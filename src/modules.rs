use crate::{
    constants::*,
    host::{
        BufferHandle, BuiltModuleDescriptor, In, Module, ModuleBuffersIn, ModuleBuffersOut,
        ModuleDescriptor, ModuleSettings, Out, VariadicBufferHandle,
    },
    midi::{MidiEvent, MidiEvents},
};

enum EnvelopeStage {
    Silence,
    Attack,
    Decay,
    Sustain,
    Release,
}

pub struct Envelope {
    midi_in: BufferHandle<In<MidiEvents>>,
    signal_in: BufferHandle<In<f32>>,
    signal_out: BufferHandle<Out<f32>>,
    settings: EnvelopeSettings,
    inv_attack: f32,
    inv_decay: f32,
    inv_release: f32,
    current_stage: EnvelopeStage,
    time_elapsed: f32,
    num_notes: i32,
    release_amplitude: f32,
}

pub struct EnvelopeSettings {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl ModuleSettings for Envelope {
    type Settings = EnvelopeSettings;
}

impl Module for Envelope {
    fn init(mut desc: ModuleDescriptor, settings: EnvelopeSettings) -> BuiltModuleDescriptor<Self> {
        let module = Self {
            midi_in: desc.with_buf_in::<MidiEvents>("in"),
            signal_in: desc.with_buf_in::<f32>("in"),
            signal_out: desc.with_buf_out::<f32>("out"),
            current_stage: EnvelopeStage::Silence,
            inv_attack: 1.0 / settings.attack,
            inv_decay: 1.0 / settings.decay,
            inv_release: 1.0 / settings.release,
            time_elapsed: 0.0,
            num_notes: 0,
            release_amplitude: 0.0,
            settings,
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        for ((midis, signal_in), signal_out) in buffers_in
            .get(self.midi_in)
            .iter()
            .zip(buffers_in.get(self.signal_in).iter())
            .zip(buffers_out.get(self.signal_out).iter_mut())
        {
            for midi in midis.iter() {
                if let MidiEvent::Midi { message, .. } = midi {
                    match message {
                        midly::MidiMessage::NoteOn { .. } => {
                            self.num_notes += 1;
                            self.current_stage = EnvelopeStage::Attack;
                            self.time_elapsed = 0.0;
                            self.release_amplitude = 0.0;
                        }
                        midly::MidiMessage::NoteOff { .. } => {
                            self.num_notes -= 1;
                            if self.num_notes == 0 {
                                match self.current_stage {
                                    EnvelopeStage::Release | EnvelopeStage::Silence => {}
                                    _ => {
                                        self.current_stage = EnvelopeStage::Release;
                                        self.time_elapsed = 0.0;
                                    }
                                }
                            } else {
                                self.current_stage = EnvelopeStage::Sustain;
                            }
                        }
                        _ => {}
                    }
                }
            }

            self.time_elapsed += SAMPLE_TIME;

            if let EnvelopeStage::Attack = self.current_stage {
                if self.time_elapsed >= self.settings.attack {
                    self.time_elapsed -= self.settings.attack;
                    self.current_stage = EnvelopeStage::Decay;
                } else {
                    self.release_amplitude = self.time_elapsed * self.inv_attack;
                    *signal_out = signal_in * self.release_amplitude;
                    continue;
                }
            }
            if let EnvelopeStage::Decay = self.current_stage {
                if self.time_elapsed >= self.settings.decay {
                    self.current_stage = EnvelopeStage::Sustain;
                } else {
                    self.release_amplitude = (1.0 - self.settings.sustain)
                        * (1.0 - self.time_elapsed * self.inv_decay)
                        + self.settings.sustain;
                    *signal_out = signal_in * self.release_amplitude;
                    continue;
                }
            }
            if let EnvelopeStage::Sustain = self.current_stage {
                self.release_amplitude = self.settings.sustain;
                *signal_out = signal_in * self.release_amplitude;
                continue;
            }
            if let EnvelopeStage::Release = self.current_stage {
                if self.time_elapsed >= self.settings.release {
                    self.current_stage = EnvelopeStage::Silence;
                } else {
                    *signal_out = signal_in
                        * self.release_amplitude
                        * (1.0 - self.time_elapsed * self.inv_release);
                    continue;
                }
            }

            *signal_out = 0.0;
        }
    }
}

pub struct Op {
    signal_in: VariadicBufferHandle<In<f32>>,
    signal_out: BufferHandle<Out<f32>>,
    op: OpType,
}

#[derive(Clone, Copy)]
pub enum OpType {
    Add,
    Multiply,
    Negate,
}

impl ModuleSettings for Op {
    type Settings = OpType;
}

impl Module for Op {
    fn init(mut desc: ModuleDescriptor, operation: OpType) -> BuiltModuleDescriptor<Self> {
        let module = Self {
            op: operation,
            signal_in: desc.with_variadic_buf_in_default(
                "in",
                match operation {
                    OpType::Multiply => 1.0,
                    _ => 0.0,
                },
            ),
            signal_out: desc.with_buf_out::<f32>("out"),
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let signal_out = buffers_out.get(self.signal_out);
        match self.op {
            OpType::Add => {
                for val_out in signal_out.iter_mut() {
                    *val_out = 0.0;
                }
                for buf_in in buffers_in.get_iter(self.signal_in) {
                    for (val_in, val_out) in buf_in.iter().zip(signal_out.iter_mut()) {
                        *val_out += val_in;
                    }
                }
            }
            OpType::Multiply => {
                for val_out in signal_out.iter_mut() {
                    *val_out = 1.0;
                }
                for buf_in in buffers_in.get_iter(self.signal_in) {
                    for (val_in, val_out) in buf_in.iter().zip(signal_out.iter_mut()) {
                        *val_out *= val_in;
                    }
                }
            }
            OpType::Negate => {
                for val_out in signal_out.iter_mut() {
                    *val_out = 0.0;
                }
                for buf_in in buffers_in.get_iter(self.signal_in) {
                    for (val_in, val_out) in buf_in.iter().zip(signal_out.iter_mut()) {
                        *val_out -= val_in;
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct OscillatorData {
    velocity: u8,
    semitone: f32,
    bend: f32,
    frequency: f32,
    wavetable: Vec<f32>,
    wavetable_index: f32,
}

pub struct Oscillator {
    midi_in: BufferHandle<In<MidiEvents>>,
    pitch_shift: BufferHandle<In<f32>>,
    vel_amt: BufferHandle<In<f32>>,
    freq_mod: BufferHandle<In<f32>>,
    signal_out: BufferHandle<Out<f32>>,
    data: OscillatorData,
}

impl Oscillator {
    fn sine(table_len: usize) -> Vec<f32> {
        let inv_len = 1.0 / table_len as f32;
        (0..table_len)
            .map(|i| (i as f32 * std::f32::consts::TAU * inv_len).sin())
            .collect()
    }

    fn saw(table_len: usize) -> Vec<f32> {
        let inv_len = 1.0 / table_len as f32;
        (0..table_len).map(|i| i as f32 * inv_len).collect()
    }

    fn triangle(table_len: usize) -> Vec<f32> {
        let inv_len = 1.0 / table_len as f32;
        (0..table_len)
            .map(|i| 1.0 - 2.0 * (i as f32 * inv_len - 0.5).abs())
            .collect()
    }

    fn square() -> Vec<f32> {
        vec![-1.0, 1.0]
    }
}

pub enum OscillatorSettings {
    Sine(usize),
    Saw(usize),
    Triangle(usize),
    Square,
}

impl ModuleSettings for Oscillator {
    type Settings = OscillatorSettings;
}

impl Module for Oscillator {
    fn init(
        mut desc: ModuleDescriptor,
        settings: OscillatorSettings,
    ) -> BuiltModuleDescriptor<Self> {
        let module = Self {
            midi_in: desc.with_buf_in::<MidiEvents>("in"),
            pitch_shift: desc.with_buf_in_default::<f32>("pitch_shift", 1.0),
            vel_amt: desc.with_buf_in_default::<f32>("vel_amt", 0.0),
            freq_mod: desc.with_buf_in_default::<f32>("freq_mod", 0.0),
            signal_out: desc.with_buf_out::<f32>("out"),
            data: OscillatorData {
                wavetable: match settings {
                    OscillatorSettings::Sine(table_len) => Self::sine(table_len),
                    OscillatorSettings::Saw(table_len) => Self::saw(table_len),
                    OscillatorSettings::Triangle(table_len) => Self::triangle(table_len),
                    OscillatorSettings::Square => Self::square(),
                },
                ..Default::default()
            },
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        for (_i, ((((midis, pitch_shift), vel_amt), freq_mod), out)) in buffers_in
            .get(self.midi_in)
            .iter()
            .zip(buffers_in.get(self.pitch_shift).iter())
            .zip(buffers_in.get(self.vel_amt).iter())
            .zip(buffers_in.get(self.freq_mod).iter())
            .zip(buffers_out.get(self.signal_out).iter_mut())
            .enumerate()
        {
            let mut updated = false;
            for midi in midis.iter() {
                if let MidiEvent::Midi { message, .. } = midi {
                    match message {
                        midly::MidiMessage::NoteOn { key, vel } => {
                            self.data.velocity = vel.as_int();
                            self.data.semitone = (key.as_int() as i16 - 69) as f32;
                            self.data.wavetable_index = 0.0;
                            updated = true;
                        }
                        midly::MidiMessage::PitchBend { bend } => {
                            self.data.bend =
                                (bend.0.as_int() as i32 - 0x2000) as f32 / (0x2000 as f32);
                            updated = true;
                        }
                        _ => (),
                    }
                }
                //println!("{}", _i);
            }
            if updated {
                self.data.frequency = ((self.data.semitone + self.data.bend) / 12.0).exp2() * 440.0;
            }

            *out = self.data.wavetable[((self.data.wavetable_index + freq_mod) as usize)
                .rem_euclid(self.data.wavetable.len())]
                * (1.0 + vel_amt * ((self.data.velocity as f32 / 128.0) - 1.0));

            let table_len = self.data.wavetable.len() as f32;
            self.data.wavetable_index +=
                self.data.frequency * pitch_shift * SAMPLE_TIME * table_len;
            self.data.wavetable_index = self.data.wavetable_index.rem_euclid(table_len as f32);
        }
    }
}
