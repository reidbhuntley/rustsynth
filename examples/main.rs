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
    let start = std::time::Instant::now();

    let mut host = Host::new();

    let midi = host.create_module::<MidiInput>("midi", 0);

    let fmod_pitch_slider = host.create_module::<MidiSlider>(
        "fmod_pitch_slider",
        MidiSliderSettings {
            controller: 41,
            default: 1.0,
            min: 0.0,
            max: 4.0,
        },
    );
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(fmod_pitch_slider, "in"));

    let fmod_vol_slider = host.create_module::<MidiSlider>(
        "fmod_vol_slider",
        MidiSliderSettings {
            controller: 42,
            default: 64.0,
            min: 0.0,
            max: 128.0,
        },
    );
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(fmod_vol_slider, "in"));

    let fmod_osc = host.create_module::<Oscillator>("fmod_osc", OscillatorSettings::Square);
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(fmod_osc, "in"));
    host.link::<f32>(
        host.buf(fmod_pitch_slider, "out"),
        host.buf(fmod_osc, "pitch_shift"),
    );

    let fmod_envelope = host.create_module::<Envelope>(
        "fmod_envelope",
        EnvelopeSettings {
            attack: 0.0,
            decay: 5.0,
            sustain: 0.6,
            release: 0.2,
        },
    );
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(fmod_envelope, "in"));
    host.link::<f32>(host.buf(fmod_osc, "out"), host.buf(fmod_envelope, "in"));

    let fmod_amp = host.create_variadic_module::<Op>("fmod_amp", OpType::Multiply, 2);
    host.link::<f32>(
        host.buf(fmod_envelope, "out"),
        host.variadic_buf(fmod_amp, "in").at(0),
    );
    host.link::<f32>(
        host.buf(fmod_vol_slider, "out"),
        host.variadic_buf(fmod_amp, "in").at(1),
    );

    let carrier_osc =
        host.create_module::<Oscillator>("carrier_osc", OscillatorSettings::Sine(1024));
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(carrier_osc, "in"));
    host.link_value::<f32>(0.2, host.buf(carrier_osc, "vel_amt"));
    host.link::<f32>(host.buf(fmod_amp, "out"), host.buf(carrier_osc, "freq_mod"));

    let carrier_envelope = host.create_module::<Envelope>(
        "carrier_envelope",
        EnvelopeSettings {
            attack: 0.0,
            decay: 1.0,
            sustain: 0.6,
            release: 0.6,
        },
    );
    host.link::<MidiEvents>(host.buf(midi, "out"), host.buf(carrier_envelope, "in"));
    host.link::<f32>(
        host.buf(carrier_osc, "out"),
        host.buf(carrier_envelope, "in"),
    );

    host.link::<f32>(
        host.buf(carrier_envelope, "out"),
        host.buf(host.get_output_module(), "in"),
    );

    let dur = std::time::Instant::now().duration_since(start);
    println!("Initialized in {}s", dur.as_secs_f64());

    host.process();
}
