//! Voice search (Windows): press the microphone in the search bar and say a song, an artist or a
//! line of lyrics. It uses Windows' own speech recognition (the one behind Win+H voice typing), so
//! DK.FM carries no speech model and nothing runs until you press it: one background thread waits
//! for the few seconds you speak, Windows does the listening.

/// Whether this computer can do it (Windows only for now).
pub fn available() -> bool {
    cfg!(windows)
}

/// Listens until you stop talking (up to ~8 s of silence first) and returns what you said.
#[cfg(windows)]
pub fn listen() -> Result<String, String> {
    use windows::Foundation::TimeSpan;
    use windows::Media::SpeechRecognition::SpeechRecognitionResultStatus as St;
    let fail = |e: windows::core::Error| explain(e.code().0 as u32, &e.message());
    let rec = prepare()?;
    if let Ok(t) = rec.Timeouts() {
        let _ = t.SetInitialSilenceTimeout(TimeSpan { Duration: 80_000_000 }); // 8 s
        let _ = t.SetEndSilenceTimeout(TimeSpan { Duration: 12_000_000 }); // 1.2 s
    }
    let heard = rec.RecognizeAsync().and_then(|op| op.get()).map_err(fail)?;
    match heard.Status().map_err(fail)? {
        St::Success => {
            let text = heard.Text().map_err(fail)?.to_string();
            let text = text.trim().trim_end_matches(['.', '?', '!']).to_string();
            if text.is_empty() { Err("Didn't catch that: press the microphone and try again".into()) } else { Ok(text) }
        }
        s => Err(status(s)),
    }
}

/// A recognizer set up for searches (short phrases and names rather than sentences).
#[cfg(windows)]
fn prepare() -> Result<windows::Media::SpeechRecognition::SpeechRecognizer, String> {
    use windows::core::HSTRING;
    use windows::Media::SpeechRecognition::{SpeechRecognitionResultStatus as St, SpeechRecognitionScenario, SpeechRecognitionTopicConstraint, SpeechRecognizer};
    unsafe {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32);
    }
    let fail = |e: windows::core::Error| explain(e.code().0 as u32, &e.message());
    let rec = SpeechRecognizer::new().map_err(fail)?;
    // tuned for searches (short phrases, names) rather than dictating sentences
    let topic = SpeechRecognitionTopicConstraint::Create(SpeechRecognitionScenario::WebSearch, &HSTRING::from("music")).map_err(fail)?;
    rec.Constraints().and_then(|c| c.Append(&topic)).map_err(fail)?;
    let compiled = rec.CompileConstraintsAsync().and_then(|op| op.get()).map_err(fail)?;
    if compiled.Status().map_err(fail)? != St::Success {
        return Err(status(compiled.Status().map_err(fail)?));
    }
    Ok(rec)
}

#[cfg(not(windows))]
pub fn listen() -> Result<String, String> {
    Err("Voice search works on Windows for now".into())
}

#[cfg(windows)]
fn status(s: windows::Media::SpeechRecognition::SpeechRecognitionResultStatus) -> String {
    use windows::Media::SpeechRecognition::SpeechRecognitionResultStatus as St;
    match s {
        St::TimeoutExceeded | St::UserCanceled => "Didn't hear anything: press the microphone and say a song, an artist or a lyric".into(),
        St::AudioQualityFailure | St::MicrophoneUnavailable => "Couldn't hear the microphone: check it's plugged in and not muted".into(),
        St::NetworkFailure => "Voice search needs the internet (Windows' speech service is online)".into(),
        St::TopicLanguageNotSupported | St::GrammarLanguageMismatch => "Windows' speech recognition doesn't support your language yet: add an English speech language in Windows Settings > Time & language > Speech".into(),
        s => format!("Voice search didn't work ({})", s.0),
    }
}

/// A Windows error, in words (and what to do about it).
fn explain(code: u32, msg: &str) -> String {
    match code {
        // the online speech privacy setting is off
        0x8004_5509 => "Turn on Online speech recognition for voice search: Windows Settings > Privacy & security > Speech".into(),
        0x8007_0005 => "The microphone is blocked: allow it for desktop apps in Windows Settings > Privacy & security > Microphone".into(),
        // no speech language installed
        0x8004_5500..=0x8004_55ff => format!("Windows' speech recognition isn't set up: add a speech language in Windows Settings > Time & language > Speech ({msg})"),
        _ => format!("Voice search didn't work: {msg}"),
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// Windows lets DK.FM (a desktop app) set up its speech recognizer (no talking needed).
    #[test]
    #[ignore]
    fn recognizer_sets_up() {
        match super::prepare() {
            Ok(_) => {}
            Err(e) => panic!("{e}"),
        }
    }

    /// One real listen (nobody talking: should end with "didn't hear anything").
    #[test]
    #[ignore]
    fn listens() {
        println!("{:?}", super::listen());
    }
}
