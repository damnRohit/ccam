//! Notifications for `ccam auto`: the terminal, the desktop, and optionally an ntfy-style URL.
//!
//! Every notifier is best effort. A notification that cannot be delivered is dropped, never allowed to stop the
//! watcher that sent it.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// Prints the message with the terminal bell. In tmux the bell marks the window; an SSH app on a phone beeps or
/// vibrates while it is open.
pub struct Terminal<W: Write + Send> {
    out: Mutex<W>,
}

impl<W: Write + Send> Terminal<W> {
    pub fn new(out: W) -> Self {
        Self { out: Mutex::new(out) }
    }
}

impl<W: Write + Send> Notifier for Terminal<W> {
    fn notify(&self, title: &str, body: &str) {
        let time = jiff::Zoned::now().strftime("%H:%M:%S");
        if let Ok(mut out) = self.out.lock() {
            let _ = writeln!(out, "{time} {title}: {body}");
            let _ = out.flush();
        }
    }
}

/// Posts the body to a URL, with the title in a `Title` header: the ntfy protocol, which a self-hosted ntfy server
/// turns into a phone push notification with the ringtone chosen in the ntfy app.
pub struct Webhook {
    url: String,
    agent: ureq::Agent,
}

impl Webhook {
    pub fn new(url: String) -> Self {
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
        Self { url, agent }
    }
}

impl Notifier for Webhook {
    fn notify(&self, title: &str, body: &str) {
        let _ = self.agent.post(&self.url).header("Title", title).header("Priority", "high").send(body);
    }
}

/// A popup with sound on the machine running ccam. From WSL it reaches the Windows desktop.
pub struct Desktop;

impl Notifier for Desktop {
    fn notify(&self, title: &str, body: &str) {
        if cfg!(windows) || crate::presence::running_in_wsl() {
            windows_toast(title, body);
        } else if cfg!(target_os = "macos") {
            let script = format!(
                "display notification {} with title {} sound name \"Glass\"",
                applescript_string(body),
                applescript_string(title)
            );
            spawn_quietly(Command::new("osascript").args(["-e", &script]));
        } else {
            spawn_quietly(Command::new("notify-send").args(["-u", "critical", title, body]));
        }
    }
}

/// Sends to every notifier in turn.
pub struct Fanout(pub Vec<Box<dyn Notifier>>);

impl Notifier for Fanout {
    fn notify(&self, title: &str, body: &str) {
        for n in &self.0 {
            n.notify(title, body);
        }
    }
}

/// A Windows toast with a looping alarm sound, raised through Windows PowerShell so no helper app is needed. The
/// script goes base64-encoded so no quoting of the message text can break it.
fn windows_toast(title: &str, body: &str) {
    // Toasts need a registered app id; Windows PowerShell's is always present.
    const APP_ID: &str = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";
    let xml = format!(
        r#"<toast duration="long" scenario="reminder"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><audio src="ms-winsoundevent:Notification.Looping.Alarm2" loop="true"/><actions><action content="Dismiss" arguments="dismiss" activationType="system"/></actions></toast>"#,
        xml_escape(title),
        xml_escape(body)
    );
    let script = format!(
        "[void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime];\
         [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime];\
         $x = New-Object Windows.Data.Xml.Dom.XmlDocument; $x.LoadXml('{}');\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{}').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
        xml.replace('\'', "''"),
        APP_ID
    );
    let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64_encode(&encoded);
    spawn_quietly(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]));
}

fn spawn_quietly(command: &mut Command) {
    let _ = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Reply, TestServer};

    #[test]
    fn the_terminal_gets_the_message_and_a_bell() {
        let terminal = Terminal::new(Vec::new());
        terminal.notify("ccam", "switched a -> b");
        let out = String::from_utf8(terminal.out.into_inner().unwrap()).unwrap();
        assert!(out.contains('\x07'), "{out:?}");
        assert!(out.contains("ccam: switched a -> b"), "{out:?}");
    }

    #[test]
    fn the_webhook_posts_the_body_with_the_title_header() {
        let server = TestServer::start(vec![("/ccam", Reply::Json(200, "{}".into()))]);
        Webhook::new(server.url("/ccam")).notify("ccam switched", "a -> b at 98%");
        let request = &server.received()[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.headers["title"], "ccam switched");
        assert_eq!(request.body, "a -> b at 98%");
    }

    #[test]
    fn an_unreachable_webhook_is_ignored() {
        Webhook::new("http://127.0.0.1:9/nowhere".into()).notify("t", "b");
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn toast_text_is_escaped() {
        assert_eq!(xml_escape(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&apos;");
    }
}
