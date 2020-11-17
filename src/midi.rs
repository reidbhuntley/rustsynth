use std::{sync::mpsc, time::Instant};

use midir::{Ignore, MidiInput as MidirInput, MidiInputConnection};

use midly::live::LiveEvent as MLiveEvent;
use midly::live::SystemCommon as MSysCom;
use midly::num::*;

use crate::{
    constants::*,
    host::{Module, ModuleBuffersIn, ModuleBuffersOut, ModuleDescriptor, ModuleTypes},
};

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
    _conn_in: MidiInputConnection<()>,
    start_time: Instant,
    event_receiver: mpsc::Receiver<RawEvent>,
    event_queue: Vec<RawEvent>,
}

impl ModuleTypes for MidiInput {
    type Settings = usize;
}

impl Module for MidiInput {
    fn init(port_idx: usize) -> ModuleDescriptor<Self> {
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

        ModuleDescriptor::new(Self {
            _conn_in,
            start_time: Instant::now(),
            event_receiver: rx,
            event_queue: Vec::new(),
        })
        .with_buf_out::<MidiEvents>()
    }

    fn fill_buffers(&mut self, _buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        let start_time_new = Instant::now();
        self.event_queue.extend(self.event_receiver.try_iter());

        let buffer = &mut buffers_out.get::<MidiEvents>(0);
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
    settings: MidiSliderSettings,
    range: f32,
    current_val: f32,
}

pub struct MidiSliderSettings {
    pub controller: u8,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

impl ModuleTypes for MidiSlider {
    type Settings = MidiSliderSettings;
}

impl Module for MidiSlider {
    fn init(settings: MidiSliderSettings) -> ModuleDescriptor<Self> {
        ModuleDescriptor::new(Self {
            current_val: settings.default,
            range: settings.max - settings.min,
            settings,
        })
        .with_buf_in::<MidiEvents>()
        .with_buf_out::<f32>()
    }

    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut) {
        for (midi, out) in buffers_in
            .get::<MidiEvents>(0)
            .iter()
            .zip(buffers_out.get::<f32>(0).iter_mut())
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
