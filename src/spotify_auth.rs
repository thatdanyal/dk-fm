//! "Connect Spotify": Authorization Code + PKCE (no client secret), via a one-shot listener on
//! http://127.0.0.1:8888/callback. Gives access to the user's own library (Liked Songs, saved
//! albums, private/collaborative playlists), which Spotify only hands out with the user's consent.
use crate::net::{self, enc};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

pub const REDIRECT: &str = "http://127.0.0.1:8888/callback";
const SCOPES: &str = "user-library-read playlist-read-private playlist-read-collaborative";

pub fn b64(bytes: &[u8], url: bool) -> String {
    let t: &[u8; 64] = if url { b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_" } else { b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/" };
    let mut out = String::new();
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        out.push(t[(n >> 18) as usize & 63] as char);
        out.push(t[(n >> 12) as usize & 63] as char);
        if ch.len() > 1 {
            out.push(t[(n >> 6) as usize & 63] as char);
        } else if !url {
            out.push('=');
        }
        if ch.len() > 2 {
            out.push(t[n as usize & 63] as char);
        } else if !url {
            out.push('=');
        }
    }
    out
}

fn random_str(n: usize) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..n).map(|_| A[fastrand::usize(..A.len())] as char).collect()
}

pub struct Login {
    pub refresh: String,
    pub access: String,
    pub expires: i64,
    pub name: String,
}

const PAGE_OK: &str = "<html><body style=\"background:#120606;color:#ff4a3a;font:20px monospace;text-align:center;padding-top:20vh\">DK.FM IS CONNECTED TO SPOTIFY<br><br><span style=\"color:#aa7766\">You can close this tab and go back to DK.FM.</span></body></html>";
const PAGE_ERR: &str = "<html><body style=\"background:#120606;color:#ff4a3a;font:20px monospace;text-align:center;padding-top:20vh\">SPOTIFY DID NOT CONNECT<br><br><span style=\"color:#aa7766\">Go back to DK.FM for details.</span></body></html>";

/// Opens the browser at Spotify's consent page and waits (up to 5 minutes) for the redirect.
pub fn login(client_id: &str) -> Result<Login, String> {
    if client_id.is_empty() {
        return Err("Paste your Spotify Client ID first.".into());
    }
    let listener = TcpListener::bind("127.0.0.1:8888").map_err(|e| format!("Port 8888 is busy ({e}). Close whatever is using it and try again."))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let verifier = random_str(64);
    let challenge = b64(&Sha256::digest(verifier.as_bytes()), true);
    let state = random_str(16);
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&state={state}&code_challenge_method=S256&code_challenge={challenge}",
        enc(client_id),
        enc(SCOPES),
        enc(REDIRECT)
    );
    open::that(&url).map_err(|e| format!("Could not open your browser: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        if Instant::now() > deadline {
            return Err("Timed out waiting for Spotify. Click CONNECT to try again.".into());
        }
        let (mut s, _) = match listener.accept() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = s.set_nonblocking(false);
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.split_whitespace().nth(1).unwrap_or("");
        if !path.starts_with("/callback") {
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let q: Vec<(String, String)> = path.split_once('?').map(|(_, q)| q).unwrap_or("").split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default())).collect();
        let get = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        let ok = get("state").as_deref() == Some(state.as_str()) && get("code").is_some();
        let page = if ok { PAGE_OK } else { PAGE_ERR };
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
        if let Some(e) = get("error") {
            return Err(if e == "access_denied" { "You cancelled the Spotify login.".into() } else { format!("Spotify said: {e}") });
        }
        if !ok {
            return Err("Spotify's reply didn't match this login. Try again.".into());
        }
        break get("code").unwrap();
    };
    let j = net::post_form_json(
        "https://accounts.spotify.com/api/token",
        &[],
        &[("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", REDIRECT), ("client_id", client_id), ("code_verifier", &verifier)],
    )
    .map_err(|e| format!("Spotify login failed ({e}). Is {REDIRECT} added as a Redirect URI in your Spotify app?"))?;
    let access = j["access_token"].as_str().ok_or("Spotify login failed")?.to_string();
    let refresh = j["refresh_token"].as_str().ok_or("Spotify login failed (no refresh token)")?.to_string();
    let expires = crate::store::now_secs() + j["expires_in"].as_i64().unwrap_or(3600);
    let me = net::get_json("https://api.spotify.com/v1/me", &[("Authorization", &format!("Bearer {access}"))]).map_err(|e| {
        if e.contains("403") {
            "Spotify blocked this account (HTTP 403). In your Spotify app's dashboard, open User Management and add your Spotify email. The app owner also needs Premium.".to_string()
        } else {
            format!("Connected, but reading your profile failed ({e}).")
        }
    })?;
    let name = me["display_name"].as_str().or(me["id"].as_str()).unwrap_or("you").to_string();
    Ok(Login { refresh, access, expires, name })
}

/// Swap a refresh token for an access token. Spotify may rotate the refresh token: the new one
/// is returned so the caller can save it.
pub fn refresh(client_id: &str, refresh: &str) -> Result<(String, i64, Option<String>), String> {
    let j = net::post_form_json("https://accounts.spotify.com/api/token", &[], &[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", client_id)]).map_err(|e| {
        if e.contains("400") {
            "Your Spotify connection expired. Reconnect in Settings → Spotify.".to_string()
        } else {
            format!("Spotify sign-in refresh failed ({e}).")
        }
    })?;
    let access = j["access_token"].as_str().ok_or("Spotify refresh failed")?.to_string();
    let exp = crate::store::now_secs() + j["expires_in"].as_i64().unwrap_or(3600);
    Ok((access, exp, j["refresh_token"].as_str().map(String::from)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn b64_matches_rfc4648() {
        assert_eq!(b64(b"foobar", false), "Zm9vYmFy");
        assert_eq!(b64(b"fooba", false), "Zm9vYmE=");
        assert_eq!(b64(b"foob", true), "Zm9vYg");
        // RFC 7636 appendix B
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64(&Sha256::digest(v.as_bytes()), true), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }
}
