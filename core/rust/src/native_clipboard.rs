//! Host text clipboard. Never caches clipboard contents in the runtime.
use std::io::Write;
use std::process::{Command, Stdio};

pub(crate) fn access(text: Option<&str>) -> Result<String, String> {
    // These platform programs own desktop-session integration. Do not silently
    // substitute an in-memory clipboard when no desktop backend is available.
    let command: &[&str] = if cfg!(target_os = "macos") {
        if text.is_some() { &["/usr/bin/pbcopy"] } else { &["/usr/bin/pbpaste"] }
    } else if cfg!(target_os = "linux") {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if text.is_some() { &["wl-copy", "--type", "text/plain;charset=utf-8"] }
            else { &["wl-paste", "--no-newline"] }
        } else if text.is_some() { &["xclip", "-selection", "clipboard", "-in"] }
        else { &["xclip", "-selection", "clipboard", "-out"] }
    } else {
        return Err("OS clipboard is unsupported on this host".into());
    };
    let output = transfer(command, text)?;
    Ok(text.map(str::to_owned).unwrap_or(output))
}

fn transfer(command: &[&str], text: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(command[0]).args(&command[1..])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| format!("OS clipboard launch failed: {error}"))?;
    // Drain output while writing so a command cannot deadlock on full pipes.
    let mut stdin = child.stdin.take().unwrap();
    let input = text.unwrap_or("").as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output()
        .map_err(|error| format!("OS clipboard wait failed: {error}"));
    let written = writer.join().map_err(|_| "OS clipboard writer panicked".to_owned())?;
    let output = output?;
    if !output.status.success() {
        return Err(format!("OS clipboard command failed ({}): {}", output.status,
                           String::from_utf8_lossy(&output.stderr)));
    }
    written.map_err(|error| format!("OS clipboard write failed: {error}"))?;
    String::from_utf8(output.stdout).map_err(|error| format!("OS clipboard is not UTF-8: {error}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn isolated_transport_preserves_text_and_reports_failures() {
        for text in ["first", "λ\n日本語\n", ""] {
            assert_eq!(transfer(&["/bin/cat"], Some(text)).unwrap(), text);
        }
        let large = "λ\n".repeat(100_000);
        assert_eq!(transfer(&["/bin/cat"], Some(&large)).unwrap(), large);
        assert!(transfer(&["/usr/bin/false"], None).unwrap_err().contains("command failed"));
        assert!(transfer(&["/nonexistent/hara-clipboard-test"], None).unwrap_err().contains("launch failed"));
    }
}
