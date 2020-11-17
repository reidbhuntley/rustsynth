use crate::{
    constants::*,
    host::{Module, ModuleBuffersIn, ModuleBuffersOut, ModuleDescriptor, ModuleTypes},
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

impl ModuleTypes for Envelope {
    type Settings = EnvelopeSettings;
}

impl Module for Envelope {
    fn init(settings: EnvelopeSettings) -> ModuleDescriptor<Self> {
        ModuleDescriptor::new(Envelope {
            current_stage: EnvelopeStage::Silence,
            inv_attack: 1.0 / settings.attack,
            inv_decay: 1.0 / settings.decay,
            inv_release: 1.0 / settings.release,
            time_elapsed: 0.0,
            num_notes: 0,
            release_amplitude: 0.0,
            settings,
        })
        .with_buf_in::<MidiEvents>()
        .with_buf_in::<f32>()
        .with_buf_out::<f32>()
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let midi_in = buffers_in.get::<MidiEvents>(0);
        let buf_in = buffers_in.get::<f32>(0);
        let buf_out = buffers_out.get::<f32>(0);
        for ((midis, buf_in), buf_out) in midi_in.iter().zip(buf_in.iter()).zip(buf_out.iter_mut())
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
                    *buf_out = buf_in * self.release_amplitude;
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
                    *buf_out = buf_in * self.release_amplitude;
                    continue;
                }
            }
            if let EnvelopeStage::Sustain = self.current_stage {
                self.release_amplitude = self.settings.sustain;
                *buf_out = buf_in * self.release_amplitude;
                continue;
            }
            if let EnvelopeStage::Release = self.current_stage {
                if self.time_elapsed >= self.settings.release {
                    self.current_stage = EnvelopeStage::Silence;
                } else {
                    *buf_out = buf_in
                        * self.release_amplitude
                        * (1.0 - self.time_elapsed * self.inv_release);
                    continue;
                }
            }

            *buf_out = 0.0;
        }
    }
}

pub struct Op(OpType);

#[derive(Clone, Copy)]
pub enum OpType {
    Add(usize),
    Multiply(usize),
    Negate,
}

impl ModuleTypes for Op {
    type Settings = OpType;
}

impl Module for Op {
    fn init(operation: OpType) -> ModuleDescriptor<Self> {
        let mut desc = ModuleDescriptor::new(Self(operation)).with_buf_out::<f32>();
        match operation {
            OpType::Add(n) => {
                for _ in 0..n {
                    desc = desc.with_buf_in_default::<f32>(0.0);
                }
            }
            OpType::Multiply(n) => {
                for _ in 0..n {
                    desc = desc.with_buf_in_default::<f32>(1.0);
                }
            }
            OpType::Negate => {
                desc = desc.with_buf_in_default::<f32>(0.0);
            }
        }
        desc
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let buf_out = buffers_out.get::<f32>(0);
        match self.0 {
            OpType::Add(_) => {
                for val_out in buf_out.iter_mut() {
                    *val_out = 0.0;
                }
                for buf_in in buffers_in.iter() {
                    for (val_in, val_out) in buf_in.iter().zip(buf_out.iter_mut()) {
                        *val_out += val_in;
                    }
                }
            }
            OpType::Multiply(_) => {
                for val_out in buf_out.iter_mut() {
                    *val_out = 1.0;
                }
                for buf_in in buffers_in.iter() {
                    for (val_in, val_out) in buf_in.iter().zip(buf_out.iter_mut()) {
                        *val_out *= val_in;
                    }
                }
            }
            OpType::Negate => {
                let buf_in = buffers_in.get::<f32>(0);
                for (val_in, val_out) in buf_in.iter().zip(buf_out.iter_mut()) {
                    *val_out = -val_in;
                }
            }
        }
    }
}

#[derive(Default)]
pub struct Oscillator {
    velocity: u8,
    semitone: f32,
    bend: f32,
    frequency: f32,
    wavetable: Vec<f32>,
    wavetable_index: f32,
}

impl Oscillator {
    pub fn sine(table_len: usize) -> Self {
        let inv_len = 1.0 / table_len as f32;
        Self {
            wavetable: (0..table_len)
                .map(|i| (i as f32 * std::f32::consts::TAU * inv_len).sin())
                .collect(),
            ..Default::default()
        }
    }

    pub fn saw(table_len: usize) -> Self {
        let inv_len = 1.0 / table_len as f32;
        Self {
            wavetable: (0..table_len).map(|i| i as f32 * inv_len).collect(),
            ..Default::default()
        }
    }

    pub fn triangle(table_len: usize) -> Self {
        let inv_len = 1.0 / table_len as f32;
        Self {
            wavetable: (0..table_len)
                .map(|i| 1.0 - 2.0 * (i as f32 * inv_len - 0.5).abs())
                .collect(),
            ..Default::default()
        }
    }

    pub fn square() -> Self {
        Self {
            wavetable: vec![-1.0, 1.0],
            ..Default::default()
        }
    }
}

pub enum OscillatorSettings {
    Sine(usize),
    Saw(usize),
    Triangle(usize),
    Square,
}

impl ModuleTypes for Oscillator {
    type Settings = OscillatorSettings;
}

impl Module for Oscillator {
    fn init(settings: OscillatorSettings) -> ModuleDescriptor<Self> {
        ModuleDescriptor::new(match settings {
            OscillatorSettings::Sine(table_len) => Self::sine(table_len),
            OscillatorSettings::Saw(table_len) => Self::saw(table_len),
            OscillatorSettings::Triangle(table_len) => Self::triangle(table_len),
            OscillatorSettings::Square => Self::square(),
        })
        .with_buf_in_default::<f32>(1.0) // 0: pitch_shift
        .with_buf_in_default::<f32>(0.0) // 1: vel_amt
        .with_buf_in_default::<f32>(0.0) // 2: freq_mod
        .with_buf_in::<MidiEvents>()
        .with_buf_out::<f32>()
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let pitch_shift = buffers_in.get::<f32>(0);
        let vel_amt = buffers_in.get::<f32>(1);
        let freq_mod = buffers_in.get::<f32>(2);
        let midi = buffers_in.get::<MidiEvents>(0);
        let out = buffers_out.get::<f32>(0);
        for (_i, ((((midis, pitch_shift), vel_amt), freq_mod), out)) in midi
            .iter()
            .zip(pitch_shift.iter())
            .zip(vel_amt.iter())
            .zip(freq_mod.iter())
            .zip(out.iter_mut())
            .enumerate()
        {
            let mut updated = false;
            for midi in midis.iter() {
                if let MidiEvent::Midi { message, .. } = midi {
                    match message {
                        midly::MidiMessage::NoteOn { key, vel } => {
                            self.velocity = vel.as_int();
                            self.semitone = (key.as_int() as i16 - 69) as f32;
                            self.wavetable_index = 0.0;
                            updated = true;
                        }
                        midly::MidiMessage::PitchBend { bend } => {
                            self.bend = (bend.0.as_int() as i32 - 0x2000) as f32 / (0x2000 as f32);
                            updated = true;
                        }
                        _ => (),
                    }
                }
                //println!("{}", _i);
            }
            if updated {
                self.frequency = ((self.semitone + self.bend) / 12.0).exp2() * 440.0;
            }

            *out = self.wavetable
                [((self.wavetable_index + freq_mod) as usize).rem_euclid(self.wavetable.len())]
                * (1.0 + vel_amt * ((self.velocity as f32 / 128.0) - 1.0));

            let table_len = self.wavetable.len() as f32;
            self.wavetable_index += self.frequency * pitch_shift * SAMPLE_TIME * table_len;
            self.wavetable_index = self.wavetable_index.rem_euclid(table_len as f32);
        }
    }
}
