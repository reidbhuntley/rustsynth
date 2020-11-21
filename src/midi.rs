use std::{sync::mpsc, time::Instant};

use midir::{Ignore, MidiInput as MidirInput, MidiInputConnection};

use midly::live::LiveEvent as MLiveEvent;
use midly::live::SystemCommon as MSysCom;
use midly::num::*;

use crate::{constants::*, host::{BufferHandle, BuiltModuleDescriptor, In, Module, ModuleBuffersIn, ModuleBuffersOut, ModuleDescriptor, ModuleSettings, Out, VariadicBufferHandle}};

#[derive(Debug, Clone)]
pub enum MidiEvent {
    Midi {
        channel: u4,
        message: midly::MidiMessage,
    },
    Common(SystemCommon),
    Realtime(midly::live::SystemRealtime),
}

impl From<MLiveEvent<'_>> for MidiEvent {
    fn from(old: MLiveEvent<'_>) -> Self {
        match old {
            MLiveEvent::Midi {
                channel: x,
                message: y,
            } => Self::Midi {
                channel: x,
                message: y,
            },
            MLiveEvent::Common(x) => Self::Common(x.into()),
            MLiveEvent::Realtime(x) => Self::Realtime(x),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SystemCommon {
    SysEx(Box<[u7]>),
    MidiTimeCodeQuarterFrame(midly::live::MtcQuarterFrameMessage, u4),
    SongPosition(u14),
    SongSelect(u7),
    TuneRequest,
    Undefined(u8, Box<[u7]>),
}

impl From<MSysCom<'_>> for SystemCommon {
    fn from(old: MSysCom) -> Self {
        match old {
            MSysCom::SysEx(x) => Self::SysEx(x.into()),
            MSysCom::MidiTimeCodeQuarterFrame(x, y) => Self::MidiTimeCodeQuarterFrame(x, y),
            MSysCom::SongPosition(x) => Self::SongPosition(x),
            MSysCom::SongSelect(x) => Self::SongSelect(x),
            MSysCom::TuneRequest => Self::TuneRequest,
            MSysCom::Undefined(x, y) => Self::Undefined(x, y.into()),
        }
    }
}

#[derive(Clone)]
struct RawEvent {
    time_received: Instant,
    message: Box<[u8]>,
}

pub type MidiEvents = Vec<MidiEvent>;

pub struct MidiInput {
    buf_out: BufferHandle<Out<MidiEvents>>,
    _conn_in: MidiInputConnection<()>,
    start_time: Instant,
    event_receiver: mpsc::Receiver<RawEvent>,
    event_queue: Vec<RawEvent>,
}

impl ModuleSettings for MidiInput {
    type Settings = usize;
}

impl Module for MidiInput {
    fn init(mut desc: ModuleDescriptor, port_idx: usize, _: usize) -> BuiltModuleDescriptor<Self> {
        let mut midi_in = MidirInput::new("midir reading input").unwrap();
        midi_in.ignore(Ignore::None);

        // Get an input port
        let in_port = &midi_in.ports()[port_idx];

        let (tx, rx) = mpsc::channel();
        // conn_in needs to be a named parameter, because it needs to be kept alive until the end of the scope
        let _conn_in = midi_in
            .connect(
                in_port,
                "midir-read-input",
                move |_timestamp, message, _| {
                    tx.send(RawEvent {
                        time_received: Instant::now(),
                        message: message.into(),
                    })
                    .unwrap();
                },
                (),
            )
            .unwrap();

        let module = Self {
            buf_out: desc.with_buf_out::<MidiEvents>("out"),
            _conn_in,
            start_time: Instant::now(),
            event_receiver: rx,
            event_queue: Vec::new(),
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, _buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let start_time_new = Instant::now();
        self.event_queue.extend(self.event_receiver.try_iter());

        let buffer = &mut buffers_out.get(self.buf_out);
        for vec in buffer.iter_mut() {
            vec.clear();
        }

        let mut cutoff: Option<usize> = None;
        for (i, raw) in self.event_queue.iter().enumerate() {
            let elapsed =
                if let Some(dur) = raw.time_received.checked_duration_since(self.start_time) {
                    dur.as_secs_f32()
                } else {
                    0.0
                };

            let event = MLiveEvent::parse(&raw.message).unwrap().into();

            let idx = usize::max(0, (elapsed * SAMPLE_RATE as f32) as usize);
            if idx >= BUFFER_LEN {
                cutoff = Some(i);
                break;
            }
            buffer[idx].push(event);
        }

        if let Some(i) = cutoff {
            self.event_queue.drain(0..i);
        } else {
            self.event_queue.clear();
        }

        self.start_time = start_time_new;
    }
}

pub struct MidiSlider {
    midi_in: BufferHandle<In<MidiEvents>>,
    signal_out: BufferHandle<Out<f32>>,
    settings: MidiSliderSettings,
    range: f32,
    current_val: f32,
}

#[derive(Clone)]
pub struct MidiSliderSettings {
    pub controller: u8,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

impl ModuleSettings for MidiSlider {
    type Settings = MidiSliderSettings;
}

impl Module for MidiSlider {
    fn init(
        mut desc: ModuleDescriptor,
        settings: MidiSliderSettings,
        _: usize
    ) -> BuiltModuleDescriptor<Self> {
        let module = Self {
            midi_in: desc.with_buf_in::<MidiEvents>("in"),
            signal_out: desc.with_buf_out::<f32>("out"),
            current_val: settings.default,
            range: settings.max - settings.min,
            settings,
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        for (midi, out) in buffers_in
            .get(self.midi_in)
            .iter()
            .zip(buffers_out.get(self.signal_out).iter_mut())
        {
            let mut new_value: Option<u8> = None;
            for event in midi.iter() {
                if let MidiEvent::Midi {
                    message: midly::MidiMessage::Controller { controller, value },
                    ..
                } = event
                {
                    if self.settings.controller == controller.as_int() {
                        new_value = Some(value.as_int());
                    }
                }
            }

            if let Some(new_value) = new_value {
                self.current_val = ((new_value as f32) / 128.0) * self.range + self.settings.min;
            }

            *out = self.current_val;
        }
    }
}

pub struct MidiPoly {
    num_ports: usize,
    notes: Vec<(u8, MidiEvent)>,
    midi_in: BufferHandle<In<MidiEvents>>,
    midi_out: Vec<BufferHandle<Out<MidiEvents>>>,
    midi_out_variadic: VariadicBufferHandle<Out<MidiEvents>>
    
}

impl ModuleSettings for MidiPoly {
    type Settings = ();
}

impl Module for MidiPoly {
    fn init(mut desc: ModuleDescriptor, _settings: (), num_ports: usize) -> BuiltModuleDescriptor<Self> {
        let midi_out = desc.with_variadic_buf_out::<MidiEvents>("out");
        let module = Self {
            num_ports,
            notes: Default::default(),
            midi_in: desc.with_buf_in::<MidiEvents>("in"),
            midi_out: midi_out.iter().collect(),
            midi_out_variadic: midi_out
        };
        desc.build(module)
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        if self.num_ports == 0 {
            return;
        }

        for buffer in buffers_out.get_iter(self.midi_out_variadic) {
            for events in buffer {
                events.clear();
            }
        }

        for (i, events) in buffers_in.get(self.midi_in).iter().enumerate() {
            for event in events {
                if let MidiEvent::Midi { message, .. } = event {
                    match message {
                        midly::MidiMessage::NoteOn { key, .. } => {
                            let key = key.as_int();
                            if self.notes.iter().all(|(n, _)| *n != key) {
                                let free_buf = self.midi_out.remove(self.notes.len().min(self.num_ports - 1));
                                buffers_out.get(free_buf)[i].push(event.clone());
                                self.midi_out.insert(0, free_buf);
                                self.notes.insert(0, (key, event.clone()));
                            }
                        }
                        midly::MidiMessage::NoteOff { key, .. } => {
                            let key = key.as_int();
                            match self.notes.iter().position(|(n, _)| *n == key) {
                                Some(idx) => {
                                    self.notes.remove(idx);
                                    if idx < self.num_ports {
                                        let old_buf = self.midi_out.remove(idx);
                                        self.midi_out.push(old_buf);
                                        buffers_out.get(old_buf)[i].push(if let Some((_, on_event)) = self.notes.get(self.num_ports - 1) {
                                            on_event.clone()
                                        } else {
                                            event.clone()
                                        });
                                    }
                                }
                                None => {}
                            }
                        }
                        _ => for buf_out in buffers_out.get_iter(self.midi_out_variadic) {
                            buf_out[i].push(event.clone())
                        }
                    }
                }
            }
        }
    }
}
