pub const SAMPLE_RATE: u32 = 44100; // Hz
pub const BUFFER_LEN: usize = 512;

pub const SAMPLE_TIME: f32 = 1.0 / (SAMPLE_RATE as f32); // seconds
pub const BUFFER_TIME: f32 = SAMPLE_TIME * (BUFFER_LEN as f32); // seconds
