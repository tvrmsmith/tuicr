//! The refine call's transport: Vertex AI over HTTP, on the `ureq` already in
//! `Cargo.toml`, with a hand-rolled `authorized_user` ADC refresh.
//!
//! **No agent CLI and no subprocess.** `gd-26r.24` recorded the same model, the
//! same prompt and the same reasoning effort both ways: the CLI costs 2.0× and
//! 1.8× as much and takes 1.8× and 1.7× as long, and buys nothing that shows
//! above run-to-run noise. The transport is a 2× tax on both figures, so it is
//! not a free implementation detail.
//!
//! **One call, no vote.** Consensus of three was measured at $0.78 against
//! $0.26 for a fixture-1 accuracy *loss* (0.417 against 0.450), and a parallel
//! triple costs the slowest of three in wall clock. Determinism, the reason it
//! was bought, is supplied instead by persisting the grouping.
//!
//! The shipped arm is `claude-opus-5` at **low** effort: F1 0.427 and 0.711 for
//! $0.131 and $0.237 at 36.6s and 60.0s, ten of ten calls over both bars, zero
//! invented paths. Effort is pinned with no knob — a setting that is slower,
//! dearer and better on one fixture of two is one nobody can be told how to
//! set. The model, project and location *are* settable, from `[grouping]` or
//! from the environment, for the same reason the runner-up exists: a preview id
//! can be withdrawn, and `gemini-3-flash` is 20s and a tenth of the price away.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::{Agent, Body, http::Response};

/// The default arm. `docs/GROUPING_PASSES.md` recommends `gemini-3-flash` on
/// the numbers and the human shipped the runner-up, weighing 0.126 F1 on
/// fixture 2, 0.5 invented paths a call and a preview model id against 20
/// seconds.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

/// `global` rather than a region: at the time of recording it was the only
/// location serving `claude-opus-5` on the measured org, and Gemini 3 carries a
/// regional surcharge `global` does not.
pub const DEFAULT_LOCATION: &str = "global";

/// The reasoning effort both publishers are pinned to, read by
/// [`request_body`] at both its call sites so the value sent can never drift
/// from the value the feedback log (`gd-26r.43`) records against it.
///
/// The two publishers happen to spell the same value today: Anthropic's
/// `output_config.effort` and Gemini's `thinkingLevel` floor. That is a
/// coincidence of the current model generations, not a promise — Gemini's
/// floor is versioned independently of Anthropic's effort levels, and the
/// day they part ways this constant must be split into one per publisher
/// rather than stretched to cover both.
pub const REFINE_EFFORT: &str = "low";

/// Has to cover the answer *and* the thinking, and it is a hard stop: too small
/// truncates the JSON and the call scores as a parse failure rather than as the
/// model being wrong. The largest recorded answer is ~8K tokens.
const MAX_TOKENS: u64 = 32_000;

const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// What `[grouping]` said about the arm, if anything. `None` everywhere is the
/// shipped default, which is also what a config file with no `[grouping]`
/// section produces.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub model: Option<String>,
    pub project: Option<String>,
    pub location: Option<String>,
}

/// Where the call goes and who answers it.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub project: String,
    pub location: String,
    pub model: String,
}

impl Endpoint {
    /// Resolves the arm from the environment, then `[grouping]`, then the
    /// shipped defaults.
    ///
    /// **The environment wins over the config file**, which is the opposite of
    /// how most settings here resolve and is deliberate: these three are the
    /// ones a human changes for a single run — trying the runner-up model,
    /// billing a different project — and a config file you have to edit back
    /// afterwards is a worse tool for that than a variable on the command line.
    ///
    /// The project has two further fallbacks below the config: `GOOGLE_CLOUD_
    /// PROJECT` and the ADC file's own `quota_project_id`. They sit under
    /// `[grouping].vertex_project` because they are ambient machine state and it
    /// is an explicit statement about this tool.
    pub fn resolve(credentials: &Credentials, settings: &Settings) -> Result<Self, String> {
        Self::resolve_with(credentials, settings, &env)
    }

    /// [`Self::resolve`] over a named environment rather than the process's.
    ///
    /// `std::env::set_var` is process-global and this suite runs threaded, so a
    /// test that set a variable would flake every other test that reads one.
    /// The lookup is the seam that lets the precedence be tested at all.
    fn resolve_with(
        credentials: &Credentials,
        settings: &Settings,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let env = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let project = from(env("TUICR_VERTEX_PROJECT"), "`TUICR_VERTEX_PROJECT`")
            .or_else(|| from(settings.project.clone(), "`[grouping].vertex_project`"))
            .or_else(|| from(env("GOOGLE_CLOUD_PROJECT"), "`GOOGLE_CLOUD_PROJECT`"))
            .or_else(|| {
                from(
                    credentials.quota_project_id.clone(),
                    "the credentials' `quota_project_id`",
                )
            })
            .ok_or_else(|| {
                "no Google Cloud project: set `[grouping].vertex_project`, or run \
                 `gcloud auth application-default set-quota-project <project>`"
                    .to_string()
            })?;
        let location = from(env("TUICR_VERTEX_LOCATION"), "`TUICR_VERTEX_LOCATION`")
            .or_else(|| from(settings.location.clone(), "`[grouping].vertex_location`"))
            .unwrap_or((DEFAULT_LOCATION.to_string(), "the default location"));
        let model = (resolved_model_with(settings, lookup), "the resolved model");
        Ok(Self {
            project: segment(project)?,
            location: label(location)?,
            model: segment(model)?,
        })
    }

    /// The model this arm asks for: `TUICR_REFINE_MODEL`, then
    /// `[grouping].refine_model`, then [`DEFAULT_MODEL`].
    ///
    /// This is what the arm *asked* for, not necessarily what answered: an
    /// invalid model id still reaches here and is only rejected afterwards, by
    /// [`segment`] inside [`Self::resolve_with`], and [`Self::resolve`] can
    /// fail on the project before the model is even looked at. Kept apart from
    /// `resolve` so the feedback log (`gd-26r.43`) can record the model an
    /// attempt was made under even when the whole resolution goes on to fail.
    pub fn resolved_model(settings: &Settings) -> String {
        resolved_model_with(settings, &env)
    }

    /// Which API this model speaks. Vertex fronts both publishers and they
    /// agree on nothing below the URL: different verb, different request body,
    /// different place to find the answer text.
    fn publisher(&self) -> Result<Publisher, String> {
        if self.model.starts_with("claude-") {
            Ok(Publisher::Anthropic)
        } else if self.model.starts_with("gemini-") {
            Ok(Publisher::Google)
        } else {
            Err(format!(
                "cannot tell which Vertex publisher serves `{}`",
                self.model
            ))
        }
    }

    fn url(&self, publisher: Publisher) -> String {
        let host = if self.location == "global" {
            "https://aiplatform.googleapis.com".to_string()
        } else {
            format!("https://{}-aiplatform.googleapis.com", self.location)
        };
        format!(
            "{host}/v1/projects/{}/locations/{}/publishers/{}/models/{}:{}",
            self.project,
            self.location,
            publisher.slug(),
            self.model,
            publisher.verb(),
        )
    }
}

/// The model line of [`Endpoint::resolve_with`], pulled out so
/// [`Endpoint::resolved_model`] can resolve just the model without a
/// `Credentials` to resolve the project against.
fn resolved_model_with(settings: &Settings, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let env = |name: &str| lookup(name).filter(|value| !value.is_empty());
    from(env("TUICR_REFINE_MODEL"), "`TUICR_REFINE_MODEL`")
        .or_else(|| from(settings.model.clone(), "`[grouping].refine_model`"))
        .unwrap_or((DEFAULT_MODEL.to_string(), "the default model"))
        .0
}

/// A resolved setting paired with where it came from. Four places can supply
/// the project and each of the other two can come from a variable as easily as
/// from the file, so an error that always blamed the config key would send the
/// human to edit a line they never wrote.
fn from(value: Option<String>, source: &'static str) -> Option<(String, &'static str)> {
    value.map(|value| (value, source))
}

/// The location, which becomes the first label of the host the access token is
/// sent to. Checked here rather than at config parse time because the
/// environment overrides the file and would otherwise skip the check: a value
/// like `evil.example.com/x#` ends the label and points a `Bearer` header at a
/// host nobody named.
fn label((value, source): (String, &str)) -> Result<String, String> {
    let legal = |c: char| c.is_ascii_alphanumeric() || c == '-';
    if value.is_empty() || !value.chars().all(legal) {
        return Err(format!(
            "{source} may only contain letters, digits and `-`: `{value}`"
        ));
    }
    Ok(value)
}

/// The project and the model, which become single path segments. `@` is legal
/// there and Vertex model ids use it to pin a version.
///
/// `.` is legal in a segment but a segment that is only dots is not a segment:
/// `..` climbs the URL path and `.` collapses, so either would send the token
/// to a path nobody named. They are refused rather than escaped, because no
/// real project or model id is spelled that way.
fn segment((value, source): (String, &str)) -> Result<String, String> {
    let legal = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@');
    let traverses = value == "." || value == ".." || value.contains("..");
    if value.is_empty() || traverses || !value.chars().all(legal) {
        return Err(format!(
            "{source} may only contain letters, digits and `-_.@`: `{value}`"
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publisher {
    Anthropic,
    Google,
}

impl Publisher {
    /// The publisher a recorded envelope names, for the measurement harness,
    /// which reads responses this module wrote the URL for.
    pub fn from_slug(slug: &str) -> Option<Publisher> {
        match slug {
            "anthropic" => Some(Publisher::Anthropic),
            "google" => Some(Publisher::Google),
            _ => None,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Publisher::Anthropic => "anthropic",
            Publisher::Google => "google",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Publisher::Anthropic => "rawPredict",
            Publisher::Google => "generateContent",
        }
    }
}

/// An `authorized_user` application-default credential, as
/// `gcloud auth application-default login` writes it.
///
/// Service accounts and external-account credentials are deliberately not read:
/// they need a signed JWT assertion or an external token source, and the map
/// records credential types beyond `authorized_user` as fog. A machine carrying
/// one gets a named failure and the heuristic partition, which is what every
/// other refine failure gets.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub quota_project_id: Option<String>,
}

impl Credentials {
    /// Reads ADC from `GOOGLE_APPLICATION_CREDENTIALS`, else from the
    /// well-known gcloud path.
    pub fn discover() -> Result<Self, String> {
        let path = Self::path()?;
        let text = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "no Google credentials at {}: {error} — run \
                 `gcloud auth application-default login`",
                path.display()
            )
        })?;
        Self::parse(&text)
    }

    fn path() -> Result<PathBuf, String> {
        if let Some(explicit) = env("GOOGLE_APPLICATION_CREDENTIALS") {
            return Ok(PathBuf::from(explicit));
        }
        let base = if let Some(config) = env("CLOUDSDK_CONFIG") {
            PathBuf::from(config)
        } else if cfg!(windows) {
            PathBuf::from(env("APPDATA").ok_or("could not determine APPDATA")?).join("gcloud")
        } else {
            PathBuf::from(env("HOME").ok_or("could not determine HOME")?)
                .join(".config")
                .join("gcloud")
        };
        Ok(base.join("application_default_credentials.json"))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("credentials are not JSON: {error}"))?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "authorized_user" {
            return Err(format!(
                "unsupported credential type `{kind}`: refine reads `authorized_user` \
                 application-default credentials only"
            ));
        }
        let field = |name: &str| {
            value
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("credentials have no `{name}`"))
        };
        Ok(Self {
            client_id: field("client_id")?,
            client_secret: field("client_secret")?,
            refresh_token: field("refresh_token")?,
            quota_project_id: value
                .get("quota_project_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

/// One refine call, start to finish: discover credentials, mint an access
/// token, POST the prompt, hand back the answer text.
///
/// `timeout` bounds the whole exchange, not one leg of it. It is the caller's
/// own budget — the blocking startup screen stops waiting on the same clock — so
/// a call that overran here would keep a socket open behind a screen that had
/// already given up. There are two requests, a token refresh and the predict,
/// and `timeout_global` bounds a single one, so the deadline is tracked here and
/// each leg is given only what is left of it.
///
/// Every `Err` is one outcome to the caller: keep the heuristic partition,
/// apply nothing partial, surface the reason.
pub fn call(prompt: &str, timeout: Duration, settings: &Settings) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let credentials = Credentials::discover()?;
    let endpoint = Endpoint::resolve(&credentials, settings)?;
    let publisher = endpoint.publisher()?;
    let url = endpoint.url(publisher);
    let token = access_token(
        &agent(remaining(deadline, "refreshing credentials")?),
        &credentials,
    )?;
    let response = post(
        &agent(remaining(deadline, "sending the prompt")?),
        &url,
        &token,
        &request_body(publisher, prompt),
    )?;
    answer_text(publisher, &response)
}

/// What is left of the caller's budget, for the leg about to be dispatched. A
/// budget already spent is named against the leg that would have run next
/// rather than against whichever leg the first caller happens to be.
fn remaining(deadline: Instant, before: &str) -> Result<Duration, String> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(format!("the refine call ran out of time before {before}"));
    }
    Ok(left)
}

/// `http_status_as_error` is off because a non-2xx body is the only place the
/// API explains itself — a disabled service, an unallowlisted model and a quota
/// denial are all bare `403`s otherwise. `https_only` because the request
/// carries a bearer token.
fn agent(within: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(within))
        .https_only(true)
        .http_status_as_error(false)
        .build()
        .into()
}

/// The body of a response, with a non-2xx turned into the API's own words.
fn body_json(response: Response<Body>, what: &str) -> Result<Value, String> {
    let status = response.status();
    let body = response
        .into_body()
        .read_json::<Value>()
        .map_err(|error| format!("{what} returned no JSON: {error}"));
    if status.is_success() {
        return body;
    }
    let detail = body
        .ok()
        .and_then(|value| api_message(&value))
        .unwrap_or_else(|| "no message in the response body".to_string());
    Err(format!(
        "{what} failed (HTTP {}): {detail}",
        status.as_u16()
    ))
}

/// How Google spells an error: `error.message` on Vertex, `error_description`
/// on the token endpoint.
fn api_message(body: &Value) -> Option<String> {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| body.get("error_description").and_then(Value::as_str))
        .or_else(|| body.get("error").and_then(Value::as_str))
        .map(str::to_string)
}

/// Minted per call rather than cached. A token outlives one refine comfortably,
/// and refine runs once per review, so there is nothing to reuse it for.
fn access_token(agent: &Agent, credentials: &Credentials) -> Result<String, String> {
    let response = agent
        .post(TOKEN_ENDPOINT)
        .send_form([
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
            ("refresh_token", credentials.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .map_err(|error| format!("could not refresh Google credentials: {error}"))?;
    let body = body_json(response, "the credential refresh")?;
    body.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "credential refresh returned no `access_token`".to_string())
}

fn post(agent: &Agent, url: &str, token: &str, body: &Value) -> Result<Value, String> {
    let response = agent
        .post(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send_json(body)
        .map_err(|error| format!("the refine call failed: {error}"))?;
    body_json(response, "the refine call")
}

/// The request, per publisher.
///
/// Reasoning is most of the bill (`gd-26r.24`), so the effort setting is the
/// arm's defining property and it is pinned here rather than exposed.
/// `claude-opus-5` rejects a token budget outright and wants
/// `thinking.type = adaptive` with `output_config.effort`; Gemini 3 cannot be
/// told to stop thinking at all and takes a `thinkingLevel` whose floor is
/// `low`. Both spell the same knob the CLI spells `--effort`.
fn request_body(publisher: Publisher, prompt: &str) -> Value {
    match publisher {
        Publisher::Anthropic => json!({
            "anthropic_version": "vertex-2023-10-16",
            "max_tokens": MAX_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": REFINE_EFFORT},
        }),
        Publisher::Google => json!({
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "generationConfig": {"thinkingConfig": {"thinkingLevel": REFINE_EFFORT}},
        }),
    }
}

/// The answer text, with the reasoning left out.
///
/// A thinking block is billed as output but is not the answer, and
/// concatenating it would put prose in front of the JSON — which the reader
/// would then spend its one retry stepping over. Anthropic marks them by block
/// type, Google by a `thought` flag.
///
/// A 200 is not an answer: a call cut off by `max_tokens` or a safety filter
/// comes back well-formed and empty or truncated. An empty body is named as the
/// call not finishing rather than passed on to be blamed on the reader.
pub fn answer_text(publisher: Publisher, response: &Value) -> Result<String, String> {
    let text: String = match publisher {
        Publisher::Anthropic => response
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| unfinished(response))?
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
        Publisher::Google => response
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .ok_or_else(|| unfinished(response))?
            .iter()
            .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
    };

    if text.trim().is_empty() {
        return Err(unfinished(response));
    }
    Ok(text)
}

fn unfinished(response: &Value) -> String {
    let reason = response
        .get("stop_reason")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .pointer("/candidates/0/finishReason")
                .and_then(Value::as_str)
        })
        .or_else(|| response.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or("unknown");
    format!("the refine call returned no answer text (`{reason}`)")
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment with nothing in it, which is what every resolution test
    /// below wants: the process's own variables are whatever the machine
    /// running the suite happens to export.
    fn bare(_name: &str) -> Option<String> {
        None
    }

    fn authorized_user(quota_project: Option<&str>) -> Credentials {
        let quota = match quota_project {
            Some(project) => format!(r#", "quota_project_id": "{project}""#),
            None => String::new(),
        };
        Credentials::parse(&format!(
            r#"{{"type": "authorized_user", "client_id": "id", "client_secret": "s",
                "refresh_token": "t"{quota}}}"#
        ))
        .expect("parses")
    }

    #[test]
    fn an_authorized_user_credential_reads_and_anything_else_is_named() {
        let credentials = Credentials::parse(
            r#"{"type": "authorized_user", "client_id": "id", "client_secret": "secret",
                "refresh_token": "token", "quota_project_id": "a-project"}"#,
        )
        .expect("authorized_user parses");
        assert_eq!(credentials.refresh_token, "token");
        assert_eq!(credentials.quota_project_id.as_deref(), Some("a-project"));

        let service_account = Credentials::parse(r#"{"type": "service_account"}"#)
            .expect_err("a service account is not read");
        assert!(service_account.contains("service_account"));
        assert!(Credentials::parse("not json").is_err());
    }

    #[test]
    fn the_endpoint_routes_each_publisher_to_its_own_verb() {
        let anthropic = Endpoint {
            project: "p".into(),
            location: "global".into(),
            model: "claude-opus-5".into(),
        };
        assert_eq!(
            anthropic.url(
                anthropic
                    .publisher()
                    .expect("a claude model is anthropic's")
            ),
            "https://aiplatform.googleapis.com/v1/projects/p/locations/global\
             /publishers/anthropic/models/claude-opus-5:rawPredict"
        );

        let gemini = Endpoint {
            project: "p".into(),
            location: "us-east5".into(),
            model: "gemini-3-flash-preview".into(),
        };
        assert_eq!(
            gemini.url(gemini.publisher().expect("a gemini model is google's")),
            "https://us-east5-aiplatform.googleapis.com/v1/projects/p/locations/us-east5\
             /publishers/google/models/gemini-3-flash-preview:generateContent"
        );

        let unknown = Endpoint {
            project: "p".into(),
            location: "global".into(),
            model: "llama-4".into(),
        };
        assert!(unknown.publisher().is_err());
    }

    /// `[grouping]` beats the shipped default, and the project keeps its
    /// ambient fallback underneath.
    #[test]
    fn the_config_names_the_arm_and_the_defaults_fill_the_rest() {
        let credentials = authorized_user(Some("adc-project"));

        let shipped =
            Endpoint::resolve_with(&credentials, &Settings::default(), &bare).expect("resolves");
        assert_eq!(shipped.model, DEFAULT_MODEL);
        assert_eq!(shipped.location, DEFAULT_LOCATION);
        assert_eq!(
            shipped.project, "adc-project",
            "with nothing configured the credentials' own project pays"
        );

        let configured = Endpoint::resolve_with(
            &credentials,
            &Settings {
                model: Some("gemini-3-flash-preview".into()),
                project: Some("configured-project".into()),
                location: Some("us-east5".into()),
            },
            &bare,
        )
        .expect("resolves");
        assert_eq!(configured.model, "gemini-3-flash-preview");
        assert_eq!(configured.location, "us-east5");
        assert_eq!(
            configured.project, "configured-project",
            "an explicit project beats the credentials' quota project"
        );
        assert_eq!(configured.publisher().unwrap(), Publisher::Google);
    }

    /// Each of the three settings takes its variable over the config file, and
    /// the project's own ambient fallback sits *below* the file rather than
    /// above it.
    #[test]
    fn a_variable_beats_the_config_file_for_every_setting() {
        let credentials = authorized_user(Some("adc-project"));
        let configured = Settings {
            model: Some("gemini-3-flash-preview".into()),
            project: Some("configured-project".into()),
            location: Some("us-east5".into()),
        };

        let overridden = Endpoint::resolve_with(&credentials, &configured, &|name| match name {
            "TUICR_VERTEX_PROJECT" => Some("env-project".into()),
            "TUICR_VERTEX_LOCATION" => Some("europe-west1".into()),
            "TUICR_REFINE_MODEL" => Some("claude-opus-5".into()),
            _ => None,
        })
        .expect("resolves");
        assert_eq!(overridden.project, "env-project");
        assert_eq!(overridden.location, "europe-west1");
        assert_eq!(overridden.model, "claude-opus-5");

        let ambient = Endpoint::resolve_with(&credentials, &configured, &|name| {
            (name == "GOOGLE_CLOUD_PROJECT").then(|| "ambient-project".into())
        })
        .expect("resolves");
        assert_eq!(
            ambient.project, "configured-project",
            "`GOOGLE_CLOUD_PROJECT` is machine state and loses to the config file"
        );

        let empty = Endpoint::resolve_with(&credentials, &configured, &|name| {
            (name == "TUICR_VERTEX_PROJECT").then(String::new)
        })
        .expect("resolves");
        assert_eq!(
            empty.project, "configured-project",
            "an exported-but-empty variable is not a value"
        );
    }

    #[test]
    fn a_machine_with_no_project_anywhere_says_which_key_to_set() {
        let credentials = authorized_user(None);
        let error = Endpoint::resolve_with(&credentials, &Settings::default(), &bare)
            .expect_err("no project");
        assert!(error.contains("[grouping].vertex_project"), "{error}");
    }

    /// The error names where the value came from. A project supplied by the
    /// credentials or by `GOOGLE_CLOUD_PROJECT` blamed on a config key sends
    /// the human to edit a line they never wrote.
    #[test]
    fn a_bad_value_is_reported_against_the_source_that_supplied_it() {
        let credentials = authorized_user(Some("bad project"));
        let error = Endpoint::resolve_with(&credentials, &Settings::default(), &bare)
            .expect_err("a project with a space is not one path segment");
        assert!(error.contains("quota_project_id"), "{error}");
        assert!(!error.contains("[grouping]"), "{error}");
    }

    /// A leg dispatched with nothing left to spend would open a socket behind a
    /// screen that had already given up, and the leg it names is the one that
    /// was about to run.
    #[test]
    fn a_spent_budget_stops_the_next_leg_and_names_it() {
        let left = remaining(
            Instant::now() + Duration::from_secs(5),
            "sending the prompt",
        )
        .expect("time is left");
        assert!(left > Duration::ZERO && left <= Duration::from_secs(5));

        let error = remaining(
            Instant::now() - Duration::from_millis(1),
            "sending the prompt",
        )
        .expect_err("the budget is gone");
        assert!(error.contains("sending the prompt"), "{error}");
    }

    /// The location is a hostname label and the token rides on that host, so a
    /// value that could end the label is refused before the request is built.
    #[test]
    fn a_location_or_project_that_could_redirect_the_token_is_refused() {
        let credentials = authorized_user(Some("adc-project"));
        let with = |settings: Settings| Endpoint::resolve_with(&credentials, &settings, &bare);

        for hostile in ["evil.example.com/x#", "us-east5.evil.example.com", "a b"] {
            let error = with(Settings {
                location: Some(hostile.into()),
                ..Settings::default()
            })
            .expect_err("a location that is not one label is refused");
            assert!(error.contains("vertex_location"), "{error}");
        }
        // `.` is a legal character in a segment, so the values that are only
        // dots have to be refused by name: `..` climbs the URL path, `.`
        // collapses, and either sends the token somewhere nobody named.
        for hostile in ["p/../other", ".", "..", "a/../b", "p..q"] {
            let error = with(Settings {
                project: Some(hostile.into()),
                ..Settings::default()
            })
            .expect_err("a project that escapes its path segment is refused");
            assert!(error.contains("vertex_project"), "{error}");
        }

        // A versioned Anthropic model id is ordinary, not hostile.
        let pinned = with(Settings {
            model: Some("claude-opus-5@20260101".into()),
            ..Settings::default()
        })
        .expect("an `@`-versioned model id resolves");
        assert!(
            pinned
                .url(pinned.publisher().expect("a claude model is anthropic's"))
                .ends_with("claude-opus-5@20260101:rawPredict")
        );
    }

    /// A non-2xx carries the only explanation the human can act on — the API
    /// disabled, the model not allowlisted, quota gone — and it is in the body.
    #[test]
    fn a_non_success_status_carries_the_apis_own_message() {
        let respond = |status: u16, body: &str| {
            Response::builder()
                .status(status)
                .body(Body::builder().mime_type("application/json").data(body))
                .expect("a response")
        };

        let denied = body_json(
            respond(
                403,
                r#"{"error": {"code": 403, "message": "Vertex AI API has not been used in project 12"}}"#,
            ),
            "the refine call",
        )
        .expect_err("403 is a failure");
        assert!(denied.contains("403"), "{denied}");
        assert!(
            denied.contains("has not been used in project 12"),
            "{denied}"
        );

        let refresh = body_json(
            respond(
                400,
                r#"{"error": "invalid_grant", "error_description": "token expired"}"#,
            ),
            "the credential refresh",
        )
        .expect_err("400 is a failure");
        assert!(refresh.contains("token expired"), "{refresh}");

        // A bare `error` string is the token endpoint's terser shape and is the
        // last of the three spellings `api_message` reads.
        let bare = body_json(
            respond(400, r#"{"error": "invalid_grant"}"#),
            "the credential refresh",
        )
        .expect_err("400 is a failure");
        assert!(bare.contains("invalid_grant"), "{bare}");

        // JSON that spells its error some fourth way says so rather than
        // quoting a field that is not an error message.
        let unquotable = body_json(respond(403, r#"{"detail": "nope"}"#), "the refine call")
            .expect_err("403 is a failure");
        assert!(unquotable.contains("403"), "{unquotable}");
        assert!(
            unquotable.contains("no message in the response body"),
            "{unquotable}"
        );

        // A body with nothing to quote still names the status rather than
        // reporting a parse problem the human cannot act on.
        let mute = body_json(respond(500, "<html>oops</html>"), "the refine call")
            .expect_err("500 is a failure");
        assert!(mute.contains("500"), "{mute}");

        let ok = body_json(respond(200, r#"{"content": []}"#), "the refine call")
            .expect("2xx reads its body");
        assert_eq!(ok, serde_json::json!({"content": []}));
    }

    /// The arm the feedback log records (`gd-26r.43`) has to ask through the
    /// same precedence the call itself resolves through, so
    /// `resolved_model_with` — the seam behind [`Endpoint::resolved_model`] —
    /// is pinned to the identical order the endpoint resolution above already
    /// covers: env beats config beats [`DEFAULT_MODEL`].
    #[test]
    fn resolved_model_follows_env_then_config_then_default() {
        assert_eq!(
            resolved_model_with(&Settings::default(), &bare),
            DEFAULT_MODEL,
            "nothing configured falls back to the shipped default"
        );

        let configured = Settings {
            model: Some("gemini-3-flash-preview".into()),
            ..Settings::default()
        };
        assert_eq!(
            resolved_model_with(&configured, &bare),
            "gemini-3-flash-preview",
            "the config file beats the default"
        );

        assert_eq!(
            resolved_model_with(&configured, &|name| {
                (name == "TUICR_REFINE_MODEL").then(|| "claude-opus-5".into())
            }),
            "claude-opus-5",
            "the environment beats the config file"
        );
    }

    #[test]
    fn the_request_pins_low_effort_on_both_publishers() {
        let anthropic = request_body(Publisher::Anthropic, "group these");
        assert_eq!(anthropic["output_config"]["effort"], "low");
        assert_eq!(anthropic["thinking"]["type"], "adaptive");
        assert_eq!(anthropic["max_tokens"], MAX_TOKENS);

        let google = request_body(Publisher::Google, "group these");
        assert_eq!(
            google["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "low"
        );
    }

    #[test]
    fn the_answer_drops_the_reasoning_and_keeps_the_json() {
        let anthropic = serde_json::json!({
            "content": [
                {"type": "thinking", "thinking": "let me consider the rules"},
                {"type": "text", "text": "{\"groups\": []}"}
            ],
            "stop_reason": "end_turn"
        });
        assert_eq!(
            answer_text(Publisher::Anthropic, &anthropic).unwrap(),
            "{\"groups\": []}"
        );

        let google = serde_json::json!({
            "candidates": [{"content": {"parts": [
                {"thought": true, "text": "reasoning"},
                {"text": "{\"groups\": []}"}
            ]}}]
        });
        assert_eq!(
            answer_text(Publisher::Google, &google).unwrap(),
            "{\"groups\": []}"
        );
    }

    #[test]
    fn a_two_hundred_with_no_answer_is_a_failure_that_names_its_reason() {
        let truncated = serde_json::json!({
            "content": [{"type": "thinking", "thinking": "..."}],
            "stop_reason": "max_tokens"
        });
        let error = answer_text(Publisher::Anthropic, &truncated).expect_err("no answer");
        assert!(error.contains("max_tokens"), "{error}");

        let filtered = serde_json::json!({
            "candidates": [{"finishReason": "SAFETY", "content": {"parts": []}}]
        });
        let error = answer_text(Publisher::Google, &filtered).expect_err("no answer");
        assert!(error.contains("SAFETY"), "{error}");
    }
}
