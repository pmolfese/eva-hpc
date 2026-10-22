use eva_core::gradient::{Config, MotionFormat};
pub use eva_core::mri::{run, triggers, Corrected, Request};
use std::sync::{atomic::AtomicU32, Arc};
pub struct State {
    pub open: bool,
    pub running: bool,
    pub config: Config,
    pub trigger: String,
    pub pns: bool,
    pub motion_format: MotionFormat,
    pub motion_name: Option<String>,
    pub timing_name: Option<String>,
    pub slices: usize,
    pub multiband: usize,
    pub timing_mode: usize,
    pub offsets_text: String,
    pub corrected: Option<Arc<Corrected>>,
    pub bypass: bool,
    pub error: Option<String>,
    pub progress: Arc<AtomicU32>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            running: false,
            config: Config::default(),
            trigger: "TREV".into(),
            pns: true,
            motion_format: MotionFormat::Afni,
            motion_name: None,
            timing_name: None,
            slices: 1,
            multiband: 1,
            timing_mode: 0,
            offsets_text: String::new(),
            corrected: None,
            bypass: false,
            error: None,
            progress: Arc::new(AtomicU32::new(0)),
        }
    }
}
