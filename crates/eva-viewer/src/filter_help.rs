//! Contextual help shared by hover tooltips and keyboard/click popovers.
use eframe::egui;

pub fn text(topic: &str) -> &'static str {
    match topic {
        "Filter Type" => "IIR uses compact recursive filters. FIR uses a windowed-sinc filter with a selectable timing mode. Auto uses IIR for low high-pass cutoffs and FIR elsewhere, according to the crossover setting.",
        "High-Pass Cutoff (Hz)" => "Attenuates slow changes below this frequency, such as baseline drift. Uncheck Enabled to leave this edge off. Stronger high-pass filtering can change slow EEG components. Both cutoffs must be below the stream's Nyquist frequency (half its sampling rate).",
        "Low-Pass Cutoff (Hz)" => "Attenuates faster activity above this frequency. Uncheck Enabled to leave this edge off. Set it above the high-pass cutoff and below half the sampling rate. Filtering PNS also requires valid cutoffs at the PNS sampling rate.",
        "Slope" => "Higher slopes produce a sharper transition and can increase ringing. These labels include both forward and backward IIR passes. High-pass and low-pass slopes are independent. Elliptic filters also depend on ripple and stopband settings.",
        "IIR Design" => "Butterworth has a smooth, monotonic magnitude response. Elliptic gives a sharper transition with controlled passband ripple and stopband attenuation. Both run forward and backward for zero phase. Butterworth is approximately −6 dB at the entered cutoff after both passes.",
        "Passband ripple (dB)" => "Allowed variation in the elliptic passband for the one-pass design. Smaller values request a flatter passband. Forward/backward application doubles the ripple in dB.",
        "Stopband attenuation (dB)" => "Requested elliptic stopband attenuation for the one-pass design. Larger values request more rejection. Forward/backward application doubles attenuation in dB. This must exceed the passband ripple.",
        "IIR high-pass crossover (Hz)" | "Crossover rule" => "In Auto mode, the high-pass edge uses IIR below the crossover. Choose At or below to include the boundary itself. The low-pass edge always uses FIR. This avoids very long FIR kernels at low high-pass cutoffs.",
        "FIR Window" => "Hamming is a fixed window. Kaiser lets you set the attenuation used to design the window and estimate its length. These settings also control FIR notch filtering when enabled.",
        "Kaiser attenuation (dB)" => "Controls the Kaiser window and tap-count estimate. Higher attenuation generally requires more taps for the same transition width, increasing processing and boundary context.",
        "Application" => "Zero phase applies FIR twice and squares its magnitude response. Single pass (delay compensated) removes the symmetric kernel's fixed delay. Forward only is causal and retains that delay, so waveform features appear later on the unchanged time axis.",
        "Design rule" => "EVA interprets the requested FIR cutoff as its one-pass −6 dB transition midpoint. EEGLAB / MNE interprets it as a passband edge and shifts the design midpoint by half the transition width. It also uses a shared tap count when both edges are FIR.",
        "Automatic transition width" | "Transition width (Hz)" => "The transition is the frequency interval over which FIR changes from passing to rejecting signal. Narrower transitions need more taps. Automatic uses 25% of the cutoff, floored at 2 Hz and limited by available frequency headroom. FIR notch uses 1 Hz when automatic is selected. Designs exceeding resource limits are rejected, not silently shortened.",
        "Line Noise" => "Off leaves line noise unchanged. Notch attenuates the selected frequency and harmonics. CleanLine estimates and subtracts sinusoids in overlapping windows. This is EVA's adaptive regression method, not the EEGLAB multitaper CleanLine algorithm.",
        "Frequency (Hz)" => "Fundamental line frequency, commonly 50 or 60 Hz. Harmonics are integer multiples of this value. Every selected harmonic must remain below half the sampling rate of each filtered stream.",
        "Harmonics" => "Includes the fundamental: 1 means only the selected frequency; 2 also includes twice that frequency. For 60 Hz and three harmonics, filtering targets 60, 120, and 180 Hz.",
        "Use FIR notch" => "Uses a windowed-sinc band-stop with a 1 Hz stopband per harmonic. Its transition width, window and application mode come from the FIR controls. Otherwise, notch uses zero-phase IIR biquads with adjustable Q.",
        "Notch Q" => "Higher Q makes the IIR notch narrower, preserving more neighboring frequencies but requiring longer settling context. Lower Q rejects a wider band.",
        "Window" => "Length of each CleanLine regression window. Windows overlap by 50% and blend with Hann weights. Short windows track changes more locally; longer windows fit the sinusoid over more data.",
        "Strength" => "Controls the CleanLine explained-energy gate and subtraction strength. Higher values make correction more aggressive. Preview the result before applying.",
        "Average reference (EEG only)" | "Reference exclusions" => "Subtracts the mean of included EEG channels after filtering. Hidden traces still contribute unless explicitly excluded here. Excluded channels still receive the reference subtraction. PNS never contributes. Known matching EEG units are required; missing samples in an included channel create reference gaps.",
        "Filter PNS" => "Applies the temporal filter settings to PNS at its own sampling rate. When off, PNS shows calibrated, unfiltered samples. Average reference remains EEG-only. Switch streams using the EEG stream / PNS stream buttons.",
        "Presets" => "Applies package-style design mechanics without changing cutoffs, line noise, reference, PNS selection or precision. EEGLAB and MNE-Python use Hamming, delay-compensated FIR with their cutoff convention. ERPLAB selects Butterworth IIR. EGI Net Station selects an elliptic/Kaiser Auto hybrid. These are approximations, not external software dependencies or guarantees of identical output.",
        "Precision" => "Float uses 32-bit recursive arithmetic; Double uses 64-bit. Auto conservatively uses Double for stability. FIR uses Double in every mode. Unsafe Float designs are rejected. Source and returned sample storage remain float32.",
        "Preview and Apply" => "Edits stay in the dialog until Preview or Apply Filter. Preview updates the displayed signal while keeping the dialog open. Cancel or Escape restores the filters that were active when the dialog opened. Apply Filter keeps the settings and closes. Remove filters clears filtering and closes. Source files are never modified.",
        _ => "",
    }
}

pub fn button(ui: &mut egui::Ui, topic: &str) {
    let message = text(topic);
    if message.is_empty() {
        return;
    }
    ui.push_id(("help", topic), |ui| {
        let response = ui
            .add(
                egui::Button::new(egui::RichText::new("?").strong())
                    .corner_radius(10.)
                    .min_size(egui::vec2(20., 20.)),
            )
            .on_hover_text(message);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Help: {topic}"),
            )
        });
        egui::Popup::from_toggle_button_response(&response).show(|ui| {
            ui.set_max_width(340.);
            ui.strong(topic);
            ui.label(message);
        });
    });
}

pub fn heading(ui: &mut egui::Ui, topic: &str) {
    ui.horizontal(|ui| {
        ui.strong(topic);
        button(ui, topic);
    });
}
