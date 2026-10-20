//! ACE-Step 1.5 asynchronous REST adapter.
//! Protocol: https://github.com/ace-step/ACE-Step-1.5/blob/main/docs/en/API.md

use std::io::Read;
use std::time::{Duration, Instant};

use reqwest::Url;
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::{Value, json};

use super::Request;
use crate::wav::{self, Audio};

pub struct AceStep {
    client: Client,
    base: Url,
    token: Option<String>,
}

fn limited_bytes(response: Response, limit: u64) -> Result<Vec<u8>, String> {
    let response = response.error_for_status().map_err(|e| {
        format!(
            "Music service HTTP error: {}",
            e.status().map_or(0, |s| s.as_u16())
        )
    })?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err("Music service response exceeds the size limit.".into());
    }
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the music service response.")?;
    if bytes.len() as u64 > limit {
        return Err("Music service response exceeds the size limit.".into());
    }
    Ok(bytes)
}

fn data(response: Response) -> Result<Value, String> {
    let body: Value = serde_json::from_slice(&limited_bytes(response, 1_048_576)?)
        .map_err(|_| "Music service returned invalid JSON.")?;
    if body["code"] != 200 || !body["error"].is_null() {
        return Err(
            "Music service rejected the request. Check the service logs and configuration.".into(),
        );
    }
    Ok(body["data"].clone())
}

impl AceStep {
    pub fn new(endpoint: &str, token: Option<String>) -> Result<Self, String> {
        let base = Url::parse(endpoint).map_err(|_| "Invalid ACE-Step service URL.")?;
        let local = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && local)
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err("Use an HTTPS service origin or a local HTTP origin, without credentials or a path.".into());
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            base,
            token,
        })
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let response = self
            .authorize(
                self.client
                    .post(self.base.join(path).map_err(|e| e.to_string())?),
            )
            .json(&body)
            .send()
            .map_err(|_| "Cannot reach ACE-Step. Check the service and try again.")?;
        data(response)
    }

    pub fn health(&self) -> Result<(), String> {
        let response = self
            .authorize(
                self.client
                    .get(self.base.join("health").map_err(|e| e.to_string())?),
            )
            .timeout(Duration::from_secs(5))
            .send()
            .map_err(|_| "ACE-Step is offline. Start the service or use the offline composer.")?;
        if data(response)?["status"] == "ok" {
            Ok(())
        } else {
            Err("ACE-Step did not report a healthy service.".into())
        }
    }

    pub fn generate(
        &self,
        request: &Request,
        cancelled: impl Fn() -> bool,
        stage: impl Fn(&str),
    ) -> Result<(Audio, Value), String> {
        request.validate()?;
        let check = || {
            if cancelled() {
                Err("Generation cancelled.".to_string())
            } else {
                Ok(())
            }
        };
        check()?;
        let parameters = json!({
            "prompt": request.caption(), "lyrics": "[Instrumental]",
            "audio_duration": request.duration_s, "bpm": request.tempo_bpm,
            "audio_format": "wav32", "batch_size": 1, "use_random_seed": false,
            "seed": request.seed, "thinking": false, "use_cot_caption": false, "use_cot_metas": false,
            "use_cot_language": false, "key_scale": "C Major", "time_signature": "4"
        });
        let job = self.post("release_task", parameters.clone())?;
        let id = job["task_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("ACE-Step did not return a task ID.")?;
        stage("waiting");
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            check()?;
            if Instant::now() >= deadline {
                return Err("ACE-Step timed out after 10 minutes. The server may still be processing the task.".into());
            }
            let result = self.post("query_result", json!({ "task_id_list": [id] }))?;
            check()?;
            let task = result
                .as_array()
                .and_then(|tasks| tasks.iter().find(|task| task["task_id"] == id))
                .ok_or("ACE-Step returned no matching task.")?;
            match task["status"].as_i64() {
                Some(0) => {
                    for _ in 0..20 {
                        check()?;
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Some(1) => {
                    let items: Value = match &task["result"] {
                        Value::String(text) => serde_json::from_str(text)
                            .map_err(|_| "Invalid ACE-Step result JSON.")?,
                        value => value.clone(),
                    };
                    let item = items
                        .as_array()
                        .and_then(|items| items.first())
                        .ok_or("ACE-Step returned no audio.")?;
                    let file = item["file"]
                        .as_str()
                        .ok_or("ACE-Step returned no audio URL.")?;
                    let url = self
                        .base
                        .join(file)
                        .map_err(|_| "Invalid ACE-Step audio URL.")?;
                    // Never send the service token to a URL supplied by another origin.
                    if url.origin() != self.base.origin()
                        || url.path() != "/v1/audio"
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(
                            "ACE-Step returned an audio URL outside its audio endpoint.".into()
                        );
                    }
                    stage("downloading");
                    let response = self
                        .authorize(self.client.get(url))
                        .send()
                        .map_err(|_| "Could not download generated music.")?;
                    let bytes = limited_bytes(response, 128 * 1024 * 1024)?;
                    check()?;
                    let audio = wav::parse_wav(&bytes)
                        .map_err(|e| format!("Invalid generated WAV: {e}"))?;
                    if audio.channels.len() > 2
                        || !(16_000..=96_000).contains(&audio.fs)
                        || (audio.duration_s() - request.duration_s as f64).abs() > 5.0
                        || audio.channels.iter().flatten().any(|s| !s.is_finite())
                    {
                        return Err("Generated audio has an unsupported format, duration or invalid samples.".into());
                    }
                    return Ok((
                        audio,
                        json!({
                            "task_id": id, "dit_model": item["dit_model"],
                            // Some servers report their configured LM name even
                            // when no LM ran. Keep that distinction in provenance.
                            "reported_lm_model": item["lm_model"], "seed_value": item["seed_value"],
                            "submitted_parameters": parameters,
                        }),
                    ));
                }
                Some(2) => {
                    return Err(
                        "ACE-Step could not generate this track. Try a different description."
                            .into(),
                    );
                }
                _ => return Err("ACE-Step returned an unknown task status.".into()),
            }
        }
    }
}
