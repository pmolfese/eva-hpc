use crate::filter::{Plan, Settings};
use crate::model::{reduce_from, DisplayData, ViewKey};
use eframe::egui;
use eva_mff::Recording;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
};

pub enum Job {
    Open(PathBuf),
    View(Arc<Recording>, ViewKey, Option<Arc<Vec<Vec<f32>>>>),
    Sample(
        Arc<Recording>,
        usize,
        u64,
        Settings,
        Option<Arc<Vec<Vec<f32>>>>,
    ),
    Correct(Arc<Recording>, Box<crate::mri::Request>),
}
pub enum Output {
    Open(Arc<Recording>),
    View(Box<DisplayData>),
    Corrected(Arc<crate::mri::Corrected>),
    Sample {
        channel: usize,
        sample: u64,
        value: f32,
    },
}
pub struct Reply {
    pub id: u64,
    pub result: Result<Output, String>,
}
struct Mailbox {
    pending: Option<(u64, Job)>,
    reply: Option<Reply>,
    stop: bool,
}
pub struct Worker {
    state: Arc<(Mutex<Mailbox>, Condvar)>,
    generation: Arc<AtomicU64>,
}
impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let state = Arc::new((
            Mutex::new(Mailbox {
                pending: None,
                reply: None,
                stop: false,
            }),
            Condvar::new(),
        ));
        let generation = Arc::new(AtomicU64::new(0));
        let shared = state.clone();
        let gen = generation.clone();
        thread::Builder::new()
            .name("eva-mff-reader".into())
            .spawn(move || loop {
                let (id, job) = {
                    let (lock, cv) = &*shared;
                    let mut s = lock.lock().unwrap();
                    while s.pending.is_none() && !s.stop {
                        s = cv.wait(s).unwrap();
                    }
                    if s.stop {
                        return;
                    }
                    s.pending.take().unwrap()
                };
                let result = match job {
                    Job::Open(path) => Recording::open(path)
                        .map(|r| Output::Open(Arc::new(r)))
                        .map_err(|e| e.to_string()),
                    Job::Correct(r, request) => {
                        crate::mri::run(&r, &request, &|| gen.load(Ordering::Relaxed) != id)
                            .map(|v| Output::Corrected(Arc::new(v)))
                    }
                    Job::View(r, key, source) => {
                        reduce_from(&r, key, &gen, id, source).map(|v| Output::View(Box::new(v)))
                    }
                    Job::Sample(r, channel, sample, filters, source) => {
                        Plan::new(&filters, r.metadata().sample_rate)
                            .and_then(|p| {
                                p.with_source(source).read(
                                    &r,
                                    channel,
                                    sample..sample + 1,
                                    &gen,
                                    id,
                                )
                            })
                            .map(|w| Output::Sample {
                                channel,
                                sample,
                                value: w[0],
                            })
                    }
                };
                let mut s = shared.0.lock().unwrap();
                if gen.load(Ordering::Relaxed) == id && !s.stop {
                    s.reply = Some(Reply { id, result });
                    ctx.request_repaint();
                }
            })
            .expect("reader worker thread");
        Self { state, generation }
    }
    pub fn submit(&self, job: Job) -> u64 {
        let mut s = self.state.0.lock().unwrap();
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        s.pending = Some((id, job));
        s.reply = None;
        self.state.1.notify_one();
        id
    }
    pub fn cancel(&self) {
        let mut s = self.state.0.lock().unwrap();
        self.generation.fetch_add(1, Ordering::Relaxed);
        s.pending = None;
        s.reply = None;
    }
    pub fn take(&self) -> Option<Reply> {
        self.state.0.lock().unwrap().reply.take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.state.0.lock().unwrap().stop = true;
        self.state.1.notify_one();
    }
}
