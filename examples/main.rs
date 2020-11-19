use rustsynth::{
    host::Host,
    midi::MidiEvents,
    midi::MidiInput,
    midi::MidiSlider,
    midi::MidiSliderSettings,
    modules::Envelope,
    modules::EnvelopeSettings,
    modules::{Op, OpType, Oscillator, OscillatorSettings},
};

use std::error::Error;

fn main() {
    match run() {
        Ok(_) => (),
        Err(err) => println!("Error: {}", err),
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut host = Host::new();

    host.create_module::<MidiInput>("midi", 0);

    host.create_module::<MidiSlider>(
        "fmod_pitch_slider",
        MidiSliderSettings {
            controller: 41,
            default: 1.0,
            min: 0.0,
            max: 4.0,
        },
    );
    host.link::<MidiEvents>("midi", "out", "fmod_pitch_slider", "in");

    host.create_module::<MidiSlider>(
        "fmod_vol_slider",
        MidiSliderSettings {
            controller: 42,
            default: 64.0,
            min: 0.0,
            max: 128.0,
        },
    );
    host.link::<MidiEvents>("midi", "out", "fmod_vol_slider", "in");

    host.create_module::<Oscillator>("fmod_osc", OscillatorSettings::Square);
    host.link::<MidiEvents>("midi", "out", "fmod_osc", "in");
    host.link::<f32>("fmod_pitch_slider", "out", "fmod_osc", "pitch_shift");

    host.create_module::<Envelope>(
        "fmod_envelope",
        EnvelopeSettings {
            attack: 0.0,
            decay: 5.0,
            sustain: 0.6,
            release: 0.2,
        },
    );
    host.link::<MidiEvents>("midi", "out", "fmod_envelope", "in");
    host.link::<f32>("fmod_osc", "out", "fmod_envelope", "in");

    host.create_module::<Op>("fmod_amp", OpType::Multiply(2));
    host.link::<f32>("fmod_envelope", "out", "fmod_amp", "0");
    host.link::<f32>("fmod_vol_slider", "out", "fmod_amp", "1");

    host.create_module::<Oscillator>("carrier_osc", OscillatorSettings::Sine(1024));
    host.link::<MidiEvents>("midi", "out", "carrier_osc", "in");
    host.link_value::<f32>(0.2, "carrier_osc", "vel_amt");
    host.link::<f32>("fmod_amp", "out", "carrier_osc", "freq_mod");

    host.create_module::<Envelope>(
        "carrier_envelope",
        EnvelopeSettings {
            attack: 0.0,
            decay: 1.0,
            sustain: 0.6,
            release: 0.6,
        },
    );
    host.link::<MidiEvents>("midi", "out", "carrier_envelope", "in");
    host.link::<f32>("carrier_osc", "out", "carrier_envelope", "in");

    host.link::<f32>("carrier_envelope", "out", "audio_out", "in");

    host.process();
}
