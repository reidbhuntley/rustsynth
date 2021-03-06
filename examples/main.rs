use anyhow::Result;

use rustsynth::{
    host::Host,
    midi::MidiEvents,
    midi::MidiInput,
    midi::MidiPoly,
    midi::MidiSlider,
    midi::MidiSliderSettings,
    modules::Envelope,
    modules::EnvelopeSettings,
    modules::{Op, OpType, Oscillator, OscillatorSettings},
};

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {:?}", err);
    }
}

fn run() -> Result<()> {
    let start = std::time::Instant::now();

    let mut host = Host::new()?;

    let midi = host.create_module::<MidiInput>("midi", 0)?;

    let fmod_pitch_slider = host.create_module::<MidiSlider>(
        "fmod_pitch_slider",
        MidiSliderSettings {
            controller: 41,
            default: 1.0,
            min: 0.0,
            max: 8.0,
        },
    )?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(fmod_pitch_slider, "in")?);

    let fmod_vol_slider = host.create_module::<MidiSlider>(
        "fmod_vol_slider",
        MidiSliderSettings {
            controller: 42,
            default: 64.0,
            min: 0.0,
            max: 128.0,
        },
    )?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(fmod_vol_slider, "in")?);

    let carrier_atk_slider = host.create_module::<MidiSlider>(
        "carrier_atk_slider",
        MidiSliderSettings {
            controller: 43,
            default: 0.0,
            min: 0.0,
            max: 1.0,
        },
    )?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(carrier_atk_slider, "in")?);

    let carrier_rel_slider = host.create_module::<MidiSlider>(
        "carrier_rel_slider",
        MidiSliderSettings {
            controller: 44,
            default: 0.0,
            min: 0.0,
            max: 1.7,
        },
    )?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(carrier_rel_slider, "in")?);

    let carrier_vol_slider = host.create_module::<MidiSlider>(
        "carrier_vol_slider",
        MidiSliderSettings {
            controller: 7,
            default: 0.5,
            min: 0.0,
            max: 1.0,
        },
    )?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(carrier_vol_slider, "in")?);

    let group = host.create_group("group", 16, None)?;

    let voices = host.create_group_joining_module::<MidiPoly>(group, "voices", ())?;
    host.link::<MidiEvents>(host.buf(midi, "out")?, host.buf(voices.ungrouped(), "in")?);

    let fmod_osc = host.create_group_instance_module::<Oscillator>(
        group,
        "fmod_osc",
        &OscillatorSettings::Square,
    )?;
    host.link_group::<MidiEvents>(
        &host.group_joining_buf(voices, "out")?,
        &host.group_instance_buf(&fmod_osc, "in")?,
    )?;
    host.link_group_ext::<f32>(
        host.buf(fmod_pitch_slider, "out")?,
        &host.group_instance_buf(&fmod_osc, "pitch_shift")?,
    );

    let fmod_envelope = host.create_group_instance_module::<Envelope>(
        group,
        "fmod_envelope",
        &EnvelopeSettings {
            attack: 0.0,
            decay: 5.0,
            sustain: 0.6,
            release: 0.2,
        },
    )?;
    host.link_group::<MidiEvents>(
        &host.group_joining_buf(voices, "out")?,
        &host.group_instance_buf(&fmod_envelope, "in")?,
    )?;
    host.link_group_ext::<f32>(
        host.buf(carrier_atk_slider, "out")?,
        &host.group_instance_buf(&fmod_envelope, "attack")?,
    );
    host.link_group_ext::<f32>(
        host.buf(carrier_rel_slider, "out")?,
        &host.group_instance_buf(&fmod_envelope, "release")?,
    );
    host.link_group::<f32>(
        &host.group_instance_buf(&fmod_osc, "out")?,
        &host.group_instance_buf(&fmod_envelope, "in")?,
    )?;

    let fmod_amp =
        host.create_group_instance_variadic_module::<Op>(group, "fmod_amp", &OpType::Multiply, 2)?;
    host.link_group::<f32>(
        &host.group_instance_buf(&fmod_envelope, "out")?,
        &host.group_instance_variadic_buf(&fmod_amp, "in")?.at(0)?,
    )?;
    host.link_group_ext::<f32>(
        host.buf(fmod_vol_slider, "out")?,
        &host.group_instance_variadic_buf(&fmod_amp, "in")?.at(1)?,
    );

    let carrier_osc = host.create_group_instance_module::<Oscillator>(
        group,
        "carrier_osc",
        &OscillatorSettings::Sine(1024),
    )?;
    host.link_group::<MidiEvents>(
        &host.group_joining_buf(voices, "out")?,
        &host.group_instance_buf(&carrier_osc, "in")?,
    )?;
    host.link_group_value::<f32>(0.2, &host.group_instance_buf(&carrier_osc, "vel_amt")?);
    host.link_group::<f32>(
        &host.group_instance_buf(&fmod_amp, "out")?,
        &host.group_instance_buf(&carrier_osc, "freq_mod")?,
    )?;

    let carrier_envelope = host.create_group_instance_module::<Envelope>(
        group,
        "carrier_envelope",
        &EnvelopeSettings {
            attack: 0.0,
            decay: 1.0,
            sustain: 0.6,
            release: 0.6,
        },
    )?;
    host.link_group::<MidiEvents>(
        &host.group_joining_buf(voices, "out")?,
        &host.group_instance_buf(&carrier_envelope, "in")?,
    )?;
    host.link_group_ext::<f32>(
        host.buf(carrier_atk_slider, "out")?,
        &host.group_instance_buf(&carrier_envelope, "attack")?,
    );
    host.link_group_ext::<f32>(
        host.buf(carrier_rel_slider, "out")?,
        &host.group_instance_buf(&carrier_envelope, "release")?,
    );
    host.link_group::<f32>(
        &host.group_instance_buf(&carrier_osc, "out")?,
        &host.group_instance_buf(&carrier_envelope, "in")?,
    )?;

    let mixer = host.create_group_joining_module::<Op>(group, "mixer", OpType::Add)?;
    host.link_group::<f32>(
        &host.group_instance_buf(&carrier_envelope, "out")?,
        &host.group_joining_buf(mixer, "in")?,
    )?;

    let carrier_amp = host.create_variadic_module::<Op>("carrier_amp", OpType::Multiply, 2)?;
    host.link::<f32>(
        host.buf(mixer.ungrouped(), "out")?,
        host.variadic_buf(carrier_amp, "in")?.at(0)?,
    );
    host.link::<f32>(
        host.buf(carrier_vol_slider, "out")?,
        host.variadic_buf(carrier_amp, "in")?.at(1)?,
    );

    host.link::<f32>(
        host.buf(carrier_amp, "out")?,
        host.buf(host.get_output_module(), "in")?,
    );

    let dur = std::time::Instant::now().duration_since(start);
    println!("Initialized in {}s", dur.as_secs_f64());

    host.process();
}
