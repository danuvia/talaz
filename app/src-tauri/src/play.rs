//! Signs in to Google Play and downloads the game the account owns, the way a
//! phone would. The requests follow Google-Play-API (Apache-2.0), see ATTRIBUTION.md.

use std::{collections::HashMap, hash::{BuildHasher, Hasher}, path::PathBuf, process::Command, time::Duration};

use prost::Message;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{device, runtime};

const PACKAGE: &str = "com.mojang.minecraftpe";
const SIGNIN_URL: &str = "https://accounts.google.com/embedded/setup/v2/android?source=com.android.settings&xoauth_display_name=Android%20Phone&canFrp=1&canSk=1&lang=en&langCountry=en_us&hl=en-US&cc=us";
const VERSIONS_URL: &str = "https://raw.githubusercontent.com/minecraft-linux/mcpelauncher-versiondb/master/versions.";
/// The newest version known to start with the runtime on a Mac
const RECOMMENDED: &str = "1.21.114.1";
pub const ABI: &str = if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" };

// The made up tablet Talaz signs in as
const FINGERPRINT: &str = "diy/desktop/desktop:6.0/DSKTOP/desktop:user/release-keys";
const SDK: i32 = 36;
const VENDING: &str = "45.8.21-31 [0] [PR] 747433787";
const VENDING_CODE: i32 = 84582130;
const ANDROID: &str = "6.0.1 (Linux; U; Android 6.0.1; DIY_D001 Build/DSKTOP)";

/// What Talaz remembers about the Google account and the device it registered
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Account {
    email: String,
    token: String,
    mac: String,
    logging_id: i64,
    android_id: u64,
    data_version: String,
    config_token: String,
    toc_cookie: String,
}

fn account_file() -> Option<PathBuf> {
    Some(crate::root()?.join("play.json"))
}

fn load() -> Option<Account> {
    let account: Account = serde_json::from_slice(&std::fs::read(account_file()?).ok()?).ok()?;
    (!account.token.is_empty()).then_some(account)
}

// ponytail: the token sits in a file only the user can read, like mcpelauncher keeps it. Move it to the keychain for releases
fn save(account: &Account) -> Result<(), String> {
    let file = account_file().ok_or("No home folder")?;
    std::fs::create_dir_all(file.parent().ok_or("No folder")?).map_err(|err| err.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let json = serde_json::to_vec(account).map_err(|err| err.to_string())?;
    std::io::Write::write_all(&mut options.open(file).map_err(|err| err.to_string())?, &json).map_err(|err| err.to_string())
}

pub fn email() -> Option<String> {
    load().map(|account| account.email)
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new().build_hasher().finish()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// Asks Google's sign-in service for a token. `first` is the one-time token from the sign-in page
fn auth(account: &Account, service: &str, app: &str, first: Option<&str>) -> Result<HashMap<String, String>, String> {
    let android_id = format!("{:x}", account.android_id);
    let mut form = vec![("accountType", "HOSTED_OR_GOOGLE"), ("Token", first.unwrap_or(&account.token))];
    if first.is_some() {
        form.extend([("ACCESS_TOKEN", "1"), ("add_account", "1")]);
    } else {
        form.extend([("Email", account.email.as_str()), ("check_email", "1")]);
    }
    let mut req = agent().post("https://android.clients.google.com/auth").header("User-Agent", "GoogleAuth/1.4 (desktop DSKTOP); gzip").header("app", app);
    if account.android_id != 0 {
        form.push(("androidId", &android_id));
        req = req.header("device", &android_id);
    }
    let sdk = SDK.to_string();
    form.extend([
        ("has_permission", "1"),
        ("service", service),
        ("source", "android"),
        ("app", app),
        ("device_country", "us"),
        ("lang", "en_US"),
        ("sdk_version", &sdk),
        ("client_sig", "38918a453d07199354f8b19af05ec6562ced5788"),
        ("system_partition", "1"),
        ("droidguard_results", "null"),
    ]);
    let body = req.send_form(form).map_err(|err| format!("Could not reach Google: {err}"))?.body_mut().read_to_string().map_err(|err| err.to_string())?;
    let fields = parse_fields(&body);
    if let Some(error) = fields.get("Error") {
        return Err(format!("Google refused the sign-in: {error}"));
    }
    if !fields.contains_key("Auth") {
        return Err("Google sent no sign-in token".into());
    }
    Ok(fields)
}

fn parse_fields(body: &str) -> HashMap<String, String> {
    body.lines().filter_map(|line| line.split_once('=')).map(|(key, value)| (key.to_string(), value.to_string())).collect()
}

// Only the fields Talaz sends or reads; the rest of Google's messages are skipped while decoding

#[derive(Clone, PartialEq, Message)]
struct Feature {
    #[prost(string, optional, tag = "1")]
    name: Option<String>,
    #[prost(int32, optional, tag = "2")]
    gl_es_version: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
struct DeviceConfig {
    #[prost(int32, optional, tag = "1")]
    touch_screen: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    keyboard: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    navigation: Option<i32>,
    #[prost(int32, optional, tag = "4")]
    screen_layout: Option<i32>,
    #[prost(bool, optional, tag = "5")]
    has_hard_keyboard: Option<bool>,
    #[prost(bool, optional, tag = "6")]
    has_five_way_navigation: Option<bool>,
    #[prost(int32, optional, tag = "7")]
    screen_density: Option<i32>,
    #[prost(int32, optional, tag = "8")]
    gl_es_version: Option<i32>,
    #[prost(string, repeated, tag = "9")]
    shared_library: Vec<String>,
    #[prost(string, repeated, tag = "10")]
    available_feature: Vec<String>,
    #[prost(string, repeated, tag = "11")]
    native_platform: Vec<String>,
    #[prost(int32, optional, tag = "12")]
    screen_width: Option<i32>,
    #[prost(int32, optional, tag = "13")]
    screen_height: Option<i32>,
    #[prost(string, repeated, tag = "14")]
    supported_locale: Vec<String>,
    #[prost(int32, optional, tag = "18")]
    smallest_screen_width_dp: Option<i32>,
    #[prost(bool, optional, tag = "19")]
    low_ram_device: Option<bool>,
    #[prost(int64, optional, tag = "20")]
    total_memory_bytes: Option<i64>,
    #[prost(int32, optional, tag = "21")]
    max_cpu_cores: Option<i32>,
    #[prost(message, repeated, tag = "26")]
    new_available_feature: Vec<Feature>,
    #[prost(int32, optional, tag = "27")]
    screen_layout2: Option<i32>,
    #[prost(bool, optional, tag = "28")]
    keyguard_device_secure: Option<bool>,
}

/// The checkin names each feature's GL version, the config upload leaves it out
fn device_config(feature_versions: bool) -> DeviceConfig {
    let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    DeviceConfig {
        touch_screen: Some(3),
        keyboard: Some(2),
        navigation: Some(1),
        screen_layout: Some(4),
        has_hard_keyboard: Some(false),
        has_five_way_navigation: Some(false),
        screen_density: Some(420),
        gl_es_version: Some(0x30002),
        shared_library: strings(device::SHARED_LIBRARIES),
        available_feature: strings(device::FEATURES),
        native_platform: vec![ABI.into()],
        screen_width: Some(1080),
        screen_height: Some(2424),
        supported_locale: strings(device::LOCALES),
        smallest_screen_width_dp: Some(1080),
        low_ram_device: Some(false),
        total_memory_bytes: Some(4093796352),
        max_cpu_cores: Some(4),
        new_available_feature: device::FEATURES.iter().map(|name| Feature { name: Some(name.to_string()), gl_es_version: feature_versions.then_some(0) }).collect(),
        screen_layout2: Some(2),
        keyguard_device_secure: Some(false),
    }
}

#[derive(Clone, PartialEq, Message)]
struct Build {
    #[prost(string, optional, tag = "1")]
    id: Option<String>,
    #[prost(string, optional, tag = "2")]
    product: Option<String>,
    #[prost(string, optional, tag = "3")]
    carrier: Option<String>,
    #[prost(string, optional, tag = "5")]
    bootloader: Option<String>,
    #[prost(string, optional, tag = "6")]
    client: Option<String>,
    #[prost(int64, optional, tag = "7")]
    timestamp: Option<i64>,
    #[prost(int32, optional, tag = "8")]
    google_services: Option<i32>,
    #[prost(string, optional, tag = "9")]
    device: Option<String>,
    #[prost(int32, optional, tag = "10")]
    sdk_version: Option<i32>,
    #[prost(string, optional, tag = "11")]
    model: Option<String>,
    #[prost(string, optional, tag = "12")]
    manufacturer: Option<String>,
    #[prost(string, optional, tag = "13")]
    build_product: Option<String>,
    #[prost(bool, optional, tag = "14")]
    ota_installed: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct Event {
    #[prost(string, optional, tag = "1")]
    tag: Option<String>,
    #[prost(int64, optional, tag = "3")]
    time_msec: Option<i64>,
}

#[derive(Clone, PartialEq, Message)]
struct Checkin {
    #[prost(message, optional, tag = "1")]
    build: Option<Build>,
    #[prost(int64, optional, tag = "2")]
    last_checkin_msec: Option<i64>,
    #[prost(message, repeated, tag = "3")]
    event: Vec<Event>,
    #[prost(string, optional, tag = "8")]
    roaming: Option<String>,
    #[prost(int32, optional, tag = "9")]
    user_number: Option<i32>,
    #[prost(int32, optional, tag = "14")]
    device_type: Option<i32>,
    #[prost(bool, optional, tag = "18")]
    voice_capable: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct CheckinRequest {
    #[prost(message, optional, tag = "4")]
    checkin: Option<Checkin>,
    #[prost(string, optional, tag = "6")]
    locale: Option<String>,
    #[prost(int64, optional, tag = "7")]
    logging_id: Option<i64>,
    #[prost(string, repeated, tag = "9")]
    mac_addr: Vec<String>,
    #[prost(string, repeated, tag = "11")]
    account_cookie: Vec<String>,
    #[prost(string, optional, tag = "12")]
    time_zone: Option<String>,
    #[prost(int32, optional, tag = "14")]
    version: Option<i32>,
    #[prost(message, optional, tag = "18")]
    device_configuration: Option<DeviceConfig>,
    #[prost(string, repeated, tag = "19")]
    mac_addr_type: Vec<String>,
    #[prost(int32, optional, tag = "20")]
    fragment: Option<i32>,
    #[prost(int32, optional, tag = "22")]
    user_serial_number: Option<i32>,
}

#[derive(Clone, PartialEq, Message)]
struct CheckinResponse {
    #[prost(fixed64, optional, tag = "7")]
    android_id: Option<u64>,
    #[prost(string, optional, tag = "12")]
    device_data_version_info: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct ShortDescription {
    #[prost(string, optional, tag = "2")]
    brand: Option<String>,
    #[prost(string, optional, tag = "3")]
    fingerprint: Option<String>,
    #[prost(int32, optional, tag = "5")]
    user_count: Option<i32>,
    #[prost(string, optional, tag = "6")]
    manufacturer: Option<String>,
    #[prost(string, optional, tag = "7")]
    security_patch: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct Empty {}

#[derive(Clone, PartialEq, Message)]
struct UploadDeviceConfigRequest {
    #[prost(message, optional, tag = "1")]
    device_configuration: Option<DeviceConfig>,
    #[prost(message, optional, tag = "4")]
    data_service_subscriber: Option<Empty>,
    #[prost(message, optional, tag = "6")]
    short_description: Option<ShortDescription>,
}

#[derive(Clone, PartialEq, Message)]
struct UploadDeviceConfigResponse {
    #[prost(string, optional, tag = "1")]
    token: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct TocResponse {
    #[prost(string, optional, tag = "3")]
    tos_content: Option<String>,
    #[prost(string, optional, tag = "7")]
    tos_token: Option<String>,
    #[prost(bool, optional, tag = "11")]
    requires_upload_device_config: Option<bool>,
    #[prost(string, optional, tag = "22")]
    cookie: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct HttpCookie {
    #[prost(string, optional, tag = "1")]
    name: Option<String>,
    #[prost(string, optional, tag = "2")]
    value: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct Split {
    #[prost(string, optional, tag = "1")]
    id: Option<String>,
    #[prost(int64, optional, tag = "2")]
    download_size: Option<i64>,
    #[prost(string, optional, tag = "5")]
    download_url: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct DeliveryData {
    #[prost(int64, optional, tag = "1")]
    download_size: Option<i64>,
    #[prost(string, optional, tag = "3")]
    download_url: Option<String>,
    #[prost(message, repeated, tag = "5")]
    download_auth_cookie: Vec<HttpCookie>,
    #[prost(message, repeated, tag = "15")]
    split: Vec<Split>,
}

#[derive(Clone, PartialEq, Message)]
struct DeliveryResponse {
    #[prost(message, optional, tag = "2")]
    data: Option<DeliveryData>,
}

#[derive(Clone, PartialEq, Message)]
struct Payload {
    #[prost(message, optional, tag = "6")]
    toc: Option<TocResponse>,
    #[prost(message, optional, tag = "21")]
    delivery: Option<DeliveryResponse>,
    #[prost(message, optional, tag = "28")]
    upload_device_config: Option<UploadDeviceConfigResponse>,
}

#[derive(Clone, PartialEq, Message)]
struct ResponseWrapper {
    #[prost(message, optional, tag = "1")]
    payload: Option<Payload>,
}

fn checkin_request(account: &Account, cookie: Option<&str>) -> CheckinRequest {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    CheckinRequest {
        checkin: Some(Checkin {
            build: Some(Build {
                id: Some(FINGERPRINT.into()),
                product: Some("desktop".into()),
                carrier: Some("diy".into()),
                bootloader: Some("unknown".into()),
                client: Some("android-google".into()),
                timestamp: Some(1480546800),
                google_services: Some(251333035),
                device: Some("D001".into()),
                sdk_version: Some(SDK),
                model: Some("DIY_D001".into()),
                manufacturer: Some("diy".into()),
                build_product: Some("desktop".into()),
                ota_installed: Some(false),
            }),
            last_checkin_msec: Some(0),
            // a first checkin says the device started a little while ago
            event: vec![Event { tag: Some("event_log_start".into()), time_msec: Some(now - 30_000 - (random() % 270_000) as i64) }],
            roaming: Some("WIFI::".into()),
            user_number: Some(0),
            device_type: Some(3),
            voice_capable: Some(false),
        }),
        locale: Some("en_US".into()),
        logging_id: Some(account.logging_id),
        mac_addr: vec![account.mac.clone()],
        account_cookie: cookie.map(|cookie| vec![format!("[{}]", account.email), cookie.to_string()]).unwrap_or_default(),
        time_zone: Some("America/New_York".into()),
        version: Some(3),
        device_configuration: Some(device_config(true)),
        mac_addr_type: vec!["wifi".into()],
        fragment: Some(0),
        user_serial_number: Some(0),
    }
}

/// Registers the device with Google and returns its id
fn checkin(request: &CheckinRequest) -> Result<CheckinResponse, String> {
    let mut res = agent()
        .post("https://android.clients.google.com/checkin")
        .header("Content-type", "application/x-protobuffer")
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 6.0.1; DIY_D001 Build/DSKTOP)")
        .send(&request.encode_to_vec()[..])
        .map_err(|err| format!("Could not reach Google: {err}"))?;
    let body = res.body_mut().read_to_vec().map_err(|err| err.to_string())?;
    let response = CheckinResponse::decode(&body[..]).map_err(|_| format!("Google refused the device ({})", res.status()))?;
    if response.android_id.unwrap_or(0) == 0 {
        return Err(format!("Google gave the device no id ({})", res.status()));
    }
    Ok(response)
}

/// A signed in Play store session
struct Store {
    account: Account,
    market: String,
}

impl Store {
    fn open(mut account: Account) -> Result<Store, String> {
        if account.android_id == 0 {
            let cookie = auth(&account, "ac2dm", "com.google.android.gsf", None)?.remove("Auth").unwrap_or_default();
            let response = checkin(&checkin_request(&account, Some(&cookie)))?;
            account.android_id = response.android_id.unwrap_or(0);
            account.data_version = response.device_data_version_info.unwrap_or_default();
            save(&account)?;
        }
        let mut store = Store { account, market: String::new() };
        store.refresh()?;
        if store.account.toc_cookie.is_empty() || store.account.config_token.is_empty() {
            store.set_up()?;
        }
        Ok(store)
    }

    fn refresh(&mut self) -> Result<(), String> {
        self.market = auth(&self.account, "androidmarket", "com.android.vending", None)?.remove("Auth").unwrap_or_default();
        Ok(())
    }

    /// Introduces the device to the store: its terms, and what the device can run
    fn set_up(&mut self) -> Result<(), String> {
        self.request("userSettings", None, false)?;
        let toc = self.request("toc", None, true)?.toc.unwrap_or_default();
        if let Some(cookie) = toc.cookie {
            self.account.toc_cookie = cookie;
        }
        if self.request("toc", None, true)?.toc.unwrap_or_default().requires_upload_device_config == Some(true) {
            let upload = UploadDeviceConfigRequest {
                device_configuration: Some(device_config(false)),
                data_service_subscriber: Some(Empty {}),
                short_description: Some(ShortDescription {
                    brand: Some("diy".into()),
                    fingerprint: Some(FINGERPRINT.into()),
                    user_count: Some(1),
                    manufacturer: Some("diy".into()),
                    security_patch: Some(String::new()),
                }),
            };
            let response = self.request("uploadDeviceConfig", Some(("application/x-protobuf", upload.encode_to_vec())), true)?;
            self.account.config_token = response.upload_device_config.unwrap_or_default().token.unwrap_or_default();
            let toc = self.request("toc", None, true)?.toc.unwrap_or_default();
            self.account.toc_cookie = toc.cookie.ok_or("The Play store did not accept the device")?;
            if let (Some(_), Some(token)) = (toc.tos_content, toc.tos_token) {
                let form = format!("toscme=false&tost={}", urlencode(&token));
                self.request("acceptTos", Some(("application/x-www-form-urlencoded", form.into_bytes())), false)?;
            }
        }
        save(&self.account)
    }

    fn request(&mut self, path: &str, body: Option<(&str, Vec<u8>)>, config_token: bool) -> Result<Payload, String> {
        let user_agent = format!("Android-Finsky/{VENDING} (api=3,versionCode={VENDING_CODE},sdk={SDK},device=D001,hardware=desktop,product=desktop,platformVersionRelease=6.0.1,buildId=DSKTOP,isWideScreen=0,supportedAbis={ABI})");
        // the second try is after a fresh token, when the store says the old one ran out
        for attempt in 0..2 {
            let url = format!("https://android.clients.google.com/fdfe/{path}");
            let headers = [
                ("User-Agent", user_agent.clone()),
                ("Authorization", format!("GoogleLogin auth={}", self.market)),
                ("Accept-Language", "en-US".into()),
                ("X-DFE-Device-Id", format!("{:x}", self.account.android_id)),
                ("X-DFE-Content-Filters", String::new()),
                ("X-DFE-Network-Type", "4".into()),
                ("X-DFE-Request-Params", "timeoutMs=2500".into()),
                ("X-DFE-Client-Id", "am-android-google".into()),
                ("X-DFE-Device-Checkin-Consistency-Token", self.account.data_version.clone()),
                ("X-DFE-Device-Config-Token", if config_token { self.account.config_token.clone() } else { String::new() }),
                ("X-DFE-Cookie", self.account.toc_cookie.clone()),
                // ponytail: says "no experiments" every time instead of echoing the ones the store hands out
                ("X-DFE-Encoded-Targets", "CAE=".into()),
            ];
            let filled = headers.iter().filter(|(name, value)| !value.is_empty() || *name == "X-DFE-Content-Filters");
            let res = match &body {
                Some((kind, bytes)) => {
                    let mut req = agent().post(&url).header("Content-Type", *kind);
                    for (name, value) in filled {
                        req = req.header(*name, value);
                    }
                    req.send(&bytes[..])
                }
                None => {
                    let mut req = agent().get(&url);
                    for (name, value) in filled {
                        req = req.header(*name, value);
                    }
                    req.call()
                }
            };
            let mut res = res.map_err(|err| format!("Could not reach the Play store: {err}"))?;
            if res.status() == 401 && attempt == 0 {
                self.refresh()?;
                continue;
            }
            let bytes = res.body_mut().read_to_vec().map_err(|err| err.to_string())?;
            let wrapper = ResponseWrapper::decode(&bytes[..]).map_err(|_| format!("The Play store answered {} to {path}", res.status()))?;
            return wrapper.payload.ok_or(format!("The Play store answered {} to {path}", res.status()));
        }
        Err("The Play store did not accept the sign-in".into())
    }
}

fn urlencode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[derive(Serialize)]
pub struct Available {
    recommended: &'static str,
    /// (version code, name), newest first
    list: Vec<(i64, String)>,
}

/// Release versions of the game that can be downloaded
#[tauri::command(async)]
pub fn available() -> Result<Available, String> {
    let url = format!("{VERSIONS_URL}{ABI}.json.min");
    let text = ureq::get(&url).call().map_err(|err| format!("Could not load the version list: {err}"))?.body_mut().read_to_string().map_err(|err| err.to_string())?;
    Ok(Available { recommended: RECOMMENDED, list: releases(&text)? })
}

fn releases(json: &str) -> Result<Vec<(i64, String)>, String> {
    let all: Vec<(i64, String, i64)> = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let mut list: Vec<(i64, String)> = all.into_iter().filter(|(_, _, beta)| *beta == 0).map(|(code, name, _)| (code, name)).collect();
    list.sort_by_key(|(_, name)| std::cmp::Reverse(crate::version_key(name)));
    Ok(list)
}

/// Opens Google's own sign-in page. The page only ever hands Talaz a one-time token, never the password
#[tauri::command(async)]
pub fn sign_in(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("signin") {
        return window.set_focus().map_err(|err| err.to_string());
    }
    let url: tauri::Url = SIGNIN_URL.parse().map_err(|_| "Bad sign-in address")?;
    let window = WebviewWindowBuilder::new(&app, "signin", WebviewUrl::External(url.clone()))
        .title("Sign in with Google")
        .inner_size(480.0, 640.0)
        // a clean page every time, so an old sign-in can't be picked up
        .incognito(true)
        // the page expects the phone setup screen it normally runs in
        .initialization_script("window.mm = { showView() {}, setAccountIdentifier() {}, log() {} };")
        .build()
        .map_err(|err| err.to_string())?;
    std::thread::spawn(move || {
        // Google leaves the token in a cookie once the user has agreed
        while app.get_webview_window("signin").is_some() {
            std::thread::sleep(Duration::from_millis(500));
            let Ok(cookies) = window.cookies_for_url(url.clone()) else { break };
            let Some(cookie) = cookies.iter().find(|cookie| cookie.name() == "oauth_token") else { continue };
            let mut account = Account { mac: format!("{:012x}", random() & 0xfeff_ffff_ffff | 0x0200_0000_0000), logging_id: (random() >> 1) as i64, ..Default::default() };
            let result = auth(&account, "ac2dm", "com.google.android.gsf", Some(cookie.value())).and_then(|mut fields| {
                account.token = fields.remove("Token").ok_or("Google sent no account token")?;
                account.email = fields.remove("Email").ok_or("Google sent no account name")?;
                save(&account)?;
                Ok(account.email)
            });
            let _ = window.close();
            let _ = app.emit("signed-in", result);
            return;
        }
        let _ = app.emit("signed-in", Err::<String, _>("Sign-in was closed"));
    });
    Ok(())
}

#[tauri::command]
pub fn sign_out() {
    if let Some(file) = account_file() {
        let _ = std::fs::remove_file(file);
    }
}

/// Downloads one version of the game from the signed in account and unpacks it
#[tauri::command(async)]
pub fn install(app: AppHandle, code: i64, name: String) -> Result<(), String> {
    // the name becomes a folder, so only take what a version looks like
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_digit() || c == '.') || name.starts_with('.') {
        return Err("That is not a version".into());
    }
    runtime::ensure(&app)?;
    let extract = runtime::tool("mcpelauncher-extract").ok_or("The game runtime has no unpacker")?;
    runtime::progress(&app, "Asking Google Play for the game", None);
    let mut store = Store::open(load().ok_or("Sign in with Google first")?)?;
    let data = store
        .request(&format!("delivery?doc={PACKAGE}&ot=1&vc={code}&fdcf=1&fdcf=2"), None, false)?
        .delivery
        .and_then(|delivery| delivery.data)
        .filter(|data| data.download_url.as_deref().is_some_and(|url| !url.is_empty()))
        .ok_or(format!("Google Play has no Minecraft {name} for {}. The account has to own the game.", store.account.email))?;
    let cookie = data.download_auth_cookie.first().map(|c| format!("{}={}", c.name(), c.value())).ok_or("Google Play sent no download pass")?;

    let mut files = vec![("main".to_string(), data.download_url.clone().unwrap_or_default(), data.download_size())];
    files.extend(data.split.iter().map(|split| (split.id().to_string(), split.download_url().to_string(), split.download_size())));
    let total = files.iter().map(|(_, _, size)| (*size).max(0) as u64).sum();

    let root = crate::root().ok_or("No home folder")?;
    let apks = root.join("apks");
    let _ = std::fs::remove_dir_all(&apks);
    std::fs::create_dir_all(&apks).map_err(|err| err.to_string())?;
    let (mut done, mut paths) = (0, Vec::new());
    for (i, (_, url, _)) in files.iter().enumerate() {
        let res = agent()
            .get(url)
            .header("Cookie", &cookie)
            .header("Accept-Encoding", "identity")
            .header("User-Agent", format!("AndroidDownloadManager/{ANDROID}"))
            .call()
            .map_err(|err| format!("Could not download the game: {err}"))?;
        if res.status() != 200 {
            return Err(format!("Google Play answered {} to the download", res.status()));
        }
        let path = apks.join(format!("{i}.apk"));
        runtime::download(&app, &format!("Downloading Minecraft {name}"), res.into_body().into_reader(), &path, &mut done, total)?;
        paths.push(path);
    }

    runtime::progress(&app, &format!("Unpacking Minecraft {name}"), None);
    let versions = root.join("versions");
    let (part, dest) = (versions.join(format!("{name}.part")), versions.join(&name));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|err| err.to_string())?;
    let out = Command::new(extract).args(&paths).arg(&part).output().map_err(|err| err.to_string())?;
    let _ = std::fs::remove_dir_all(&apks);
    if !out.status.success() || !part.join("lib").join(ABI).join("libminecraftpe.so").exists() {
        let _ = std::fs::remove_dir_all(&part);
        return Err(format!("Could not unpack the game. {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&part, &dest).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_answer_is_read_by_line() {
        let fields = parse_fields("Token=aas_et/x=y\r\nAuth=abc\nEmail=a@b.c\n");
        assert_eq!(fields["Token"], "aas_et/x=y");
        assert_eq!(fields["Email"], "a@b.c");
    }

    #[test]
    fn releases_skip_betas_newest_first() {
        let list = releases(r#"[[1,"1.14.60.5",0],[3,"1.26.70.20",1],[2,"1.21.114.1",0]]"#).unwrap();
        assert_eq!(list, [(2, "1.21.114.1".to_string()), (1, "1.14.60.5".to_string())]);
    }

    #[test]
    fn checkin_matches_the_wire_format() {
        // field 14 (version) = 3 and field 20 (fragment) = 0 are both written out
        let bytes = CheckinRequest { version: Some(3), fragment: Some(0), ..Default::default() }.encode_to_vec();
        assert_eq!(bytes, [0x70, 3, 0xa0, 1, 0]);
    }

    /// Talks to Google, so it only runs when asked: cargo test -- --ignored
    #[test]
    #[ignore]
    fn google_registers_the_device() {
        let account = Account { mac: "02aabbccddee".into(), logging_id: 1234567, ..Default::default() };
        assert!(checkin(&checkin_request(&account, None)).unwrap().android_id.unwrap() != 0);
    }
}
