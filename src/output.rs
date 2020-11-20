use crate::{
    constants::*,
    host::{
        Buffer, BufferHandle, BuiltModuleDescriptor, In, Module, ModuleDescriptor, ModuleSettings,
    },
};

use std::sync::{Arc, Condvar, Mutex};

use rodio::Source;

#[derive(Copy, Clone)]
enum DoubleBufferName {
    BufferA,
    BufferB,
}

impl DoubleBufferName {
    fn next(&self) -> DoubleBufferName {
        match self {
            Self::BufferA => Self::BufferB,
            Self::BufferB => Self::BufferA,
        }
    }
}

struct AudioOutputState {
    index: usize,
    now_reading: DoubleBufferName,
    can_write: bool,
    out_of_samples: bool,
}

struct AudioOutputInner {
    state: Mutex<AudioOutputState>,
    can_write_condvar: Condvar,
    buffer_a: Mutex<[f32; BUFFER_LEN]>,
    buffer_b: Mutex<[f32; BUFFER_LEN]>,
}

#[derive(Clone)]
pub(crate) struct AudioOutput(Arc<AudioOutputInner>);

impl AudioOutput {
    pub fn new() -> Self {
        Self(Arc::new(AudioOutputInner {
            state: Mutex::new(AudioOutputState {
                index: 0,
                now_reading: DoubleBufferName::BufferB,
                can_write: true,
                out_of_samples: true,
            }),
            can_write_condvar: Condvar::new(),
            buffer_a: Mutex::new([0.0; BUFFER_LEN]),
            buffer_b: Mutex::new([0.0; BUFFER_LEN]),
        }))
    }

    pub fn write(&self, data: &Buffer<f32>) {
        let write_buffer_name = {
            let mut state = self.0.state.lock().unwrap();
            while !state.can_write {
                state = self.0.can_write_condvar.wait(state).unwrap();
            }
            state.out_of_samples = false;
            state.can_write = false;
            state.now_reading.next()
        };
        *self.get_buffer(write_buffer_name).try_lock().unwrap() = *data;
    }

    fn get_buffer(&self, name: DoubleBufferName) -> &Mutex<[f32; BUFFER_LEN]> {
        match name {
            DoubleBufferName::BufferA => &self.0.buffer_a,
            DoubleBufferName::BufferB => &self.0.buffer_b,
        }
    }
}

impl Iterator for AudioOutput {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let mut state = self.0.state.lock().unwrap();
        if state.out_of_samples {
            return Some(0.0);
        }

        let out = self.get_buffer(state.now_reading).try_lock().unwrap()[state.index];

        state.index += 1;
        if state.index >= BUFFER_LEN {
            state.index = 0;
            if state.can_write {
                state.out_of_samples = true;
            } else {
                state.now_reading = state.now_reading.next();
                state.can_write = true;
                self.0.can_write_condvar.notify_all();
            }
        }

        Some(out)
    }
}

impl Source for AudioOutput {
    fn current_frame_len(&self) -> Option<usize> {
        return None;
    }

    fn channels(&self) -> u16 {
        return 1;
    }

    fn sample_rate(&self) -> u32 {
        return SAMPLE_RATE;
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        return None;
    }
}

pub(crate) struct AudioOutputModule {
    signal_in: BufferHandle<In<f32>>,
    output: AudioOutput,
}

impl ModuleSettings for AudioOutputModule {
    type Settings = AudioOutput;
}

impl Module for AudioOutputModule {
    fn init(mut desc: ModuleDescriptor, output: AudioOutput) -> BuiltModuleDescriptor<Self> {
        let module = Self {
            signal_in: desc.with_buf_in::<f32>("in"),
            output,
        };
        desc.build(module)
    }

    fn fill_buffers(
        &mut self,
        buffers_in: &crate::host::ModuleBuffersIn,
        _buffers_out: &mut crate::host::ModuleBuffersOut,
    ) {
        self.output.write(buffers_in.get(self.signal_in));
    }
}
