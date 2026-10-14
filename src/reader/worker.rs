use super::{RenderedPage, book, render_pdf_page};
use iced::futures::channel::oneshot;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

type ResultPage = Result<RenderedPage, String>;
const CACHE_BYTES: usize = 64 * 1024 * 1024;

struct Job {
    generation: u64,
    page: u32,
    width: f32,
    reply: oneshot::Sender<ResultPage>,
}

/// One document and its interpreter/font caches, confined to a background thread.
#[derive(Debug)]
pub struct RenderWorker {
    jobs: mpsc::Sender<Job>,
    generation: Arc<AtomicU64>,
}

impl RenderWorker {
    pub fn new(bytes: Arc<[u8]>, extension: String) -> Self {
        let (jobs, receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::spawn(move || {
            // Keep the document and borrowing Hayro cache on the same stack/thread.
            // Neither is sent between executor threads or reopened per page.
            match extension.as_str() {
                "pdf" => match hayro::hayro_syntax::Pdf::new(Arc::new(bytes)) {
                    Ok(document) => {
                        let cache = hayro::RenderCache::new();
                        run(receiver, current, |page, width| {
                            render_pdf_page(&document, &cache, page, width)
                        });
                    }
                    Err(error) => run(receiver, current, |_, _| {
                        Err(format!("Cannot open PDF: {error:?}"))
                    }),
                },
                "epub" | "fb2" => match book::PreparedBook::new(&bytes, &extension) {
                    Ok(mut book) => run(receiver, current, |page, width| book.render(page, width)),
                    Err(error) => run(receiver, current, |_, _| Err(error.clone())),
                },
                _ => run(receiver, current, |page, width| {
                    super::render(&bytes, &extension, page, width)
                }),
            }
        });
        Self { jobs, generation }
    }

    /// Invalidates queued work when navigation or output size changes.
    pub fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Relaxed);
    }

    pub fn render(
        &self,
        generation: u64,
        page: u32,
        width: f32,
    ) -> impl Future<Output = ResultPage> + Send + 'static {
        let (reply, result) = oneshot::channel();
        let _ = self.jobs.send(Job {
            generation,
            page,
            width,
            reply,
        });
        async move {
            result
                .await
                .unwrap_or_else(|_| Err("Document renderer stopped".into()))
        }
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.generation.store(u64::MAX, Ordering::Relaxed);
    }
}

fn run(
    receiver: mpsc::Receiver<Job>,
    generation: Arc<AtomicU64>,
    mut render: impl FnMut(u32, f32) -> ResultPage,
) {
    let mut cache: VecDeque<(u32, u32, RenderedPage)> = VecDeque::new();
    let mut cached_bytes = 0;
    for job in receiver {
        if job.reply.is_canceled() || job.generation != generation.load(Ordering::Relaxed) {
            // Dropping the reply releases the waiting future immediately.
            continue;
        }
        if !job.width.is_finite() {
            let _ = job.reply.send(Err("Invalid render size".into()));
            continue;
        }
        let key = job.width.to_bits();
        let result = if let Some(index) = cache
            .iter()
            .position(|(page, width, _)| *page == job.page && *width == key)
        {
            let hit = cache.remove(index).unwrap();
            let page = hit.2.clone();
            cache.push_back(hit);
            Ok(page)
        } else {
            let result = render(job.page, job.width);
            if let Ok(page) = &result {
                while cached_bytes + page.rgba.len() > CACHE_BYTES || cache.len() >= 16 {
                    let Some((_, _, old)) = cache.pop_front() else {
                        break;
                    };
                    cached_bytes -= old.rgba.len();
                }
                cached_bytes += page.rgba.len();
                cache.push_back((job.page, key, page.clone()));
            }
            result
        };
        let _ = job.reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;

    #[test]
    fn repeated_pages_share_the_cached_bitmap_and_size_changes_rerender() {
        let worker = RenderWorker::new(
            Arc::from(include_bytes!("../../tests/fixtures/reader.pdf").as_slice()),
            "pdf".into(),
        );
        worker.set_generation(1);
        let first = block_on(worker.render(1, 0, 400.0)).unwrap();
        let next = block_on(worker.render(1, 1, 400.0)).unwrap();
        let back = block_on(worker.render(1, 0, 400.0)).unwrap();
        assert_eq!(next.page, 1);
        assert!(Arc::ptr_eq(&first.rgba, &back.rgba));
        let larger = block_on(worker.render(1, 0, 800.0)).unwrap();
        assert!(larger.width > first.width);
        worker.set_generation(2);
        assert!(block_on(worker.render(1, 2, 400.0)).is_err());
        assert_eq!(block_on(worker.render(2, 2, 400.0)).unwrap().page, 2);
    }

    #[test]
    fn obsolete_queued_pages_are_skipped_before_any_rendering() {
        let (sender, receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(2));
        let mut replies = Vec::new();
        for number in 0..10 {
            let (reply, result) = oneshot::channel();
            sender
                .send(Job {
                    generation: if number == 9 { 2 } else { 1 },
                    page: number,
                    width: 400.0,
                    reply,
                })
                .unwrap();
            replies.push(result);
        }
        drop(sender);
        let mut calls = 0;
        run(receiver, generation, |page, _| {
            calls += 1;
            Ok(RenderedPage {
                page,
                count: 10,
                width: 1,
                height: 1,
                rgba: vec![255; 4].into(),
            })
        });
        assert_eq!(calls, 1);
        assert_eq!(block_on(replies.pop().unwrap()).unwrap().unwrap().page, 9);
        for reply in replies {
            assert!(block_on(reply).is_err());
        }
    }

    #[test]
    #[ignore = "Manual performance measurement, no timing assertions"]
    fn benchmark_repeated_document_pages() {
        use std::time::Instant;
        let pdf: Arc<[u8]> =
            Arc::from(include_bytes!("../../tests/fixtures/reader.pdf").as_slice());
        let book: Arc<[u8]> = format!(
            "<FictionBook><body>{}</body></FictionBook>",
            "<p>Пример текста книги для измерения скорости загрузки страниц.</p>".repeat(300)
        )
        .into_bytes()
        .into();
        for (extension, bytes) in [("pdf", pdf), ("fb2", book)] {
            let worker = RenderWorker::new(bytes.clone(), extension.into());
            worker.set_generation(1);
            for page in 0..3 {
                block_on(worker.render(1, page, 900.0)).unwrap();
            }
            let start = Instant::now();
            for page in (0..3).cycle().take(12) {
                super::super::render(&bytes, extension, page, 900.0).unwrap();
            }
            let cold = start.elapsed();
            let start = Instant::now();
            for page in (0..3).cycle().take(12) {
                block_on(worker.render(1, page, 900.0)).unwrap();
            }
            eprintln!(
                "{extension}: 12 page requests, reopen/rerender={cold:?}, cached worker={:?}",
                start.elapsed()
            );
        }
    }
}
