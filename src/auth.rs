use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quick_xml::{Reader, events::Event};
use reqwest::{blocking::Client, redirect::Policy};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::{adb::Adb, store};

const DATA: &str = "/data/data/com.ss.android.ugc.aweme/";
const COOKIE_FILES: &[&str] = &[
    "app_webview/Default/Cookies",
    "app_webview/Cookies",
    "app_webview/Default/Network/Cookies",
    "app_webviewbytedance_com.ss.android.ugc.aweme/Default/Cookies",
    "app_webviewbytedance_com.ss.android.ugc.aweme/Cookies",
    "app_webviewbytedance_com.ss.android.ugc.aweme/Default/Network/Cookies",
];
const ACCOUNT_URL: &str = "https://aweme.snssdk.com/passport/account/info/v2/";

pub fn login_adb(serial: Option<&str>) -> Result<Value, String> {
    let adb = Adb::connect(serial)?;
    let mut device = adb.inspect()?;
    if device["root"] != true {
        return Err("读取应用登录态需要设备 root 授权".into());
    }
    let user = adb.shell("am get-current-user", false)?;
    if user.trim_ascii() != b"0" {
        return Err("当前仅支持手机主用户，请切换到主用户后导入，避免使用其他用户的账号".into());
    }
    let unlocked = adb.shell("getprop sys.user.0.ce_available", false)?;
    if unlocked.trim_ascii() == b"false" {
        return Err("应用数据尚未解锁，请先解锁手机一次再运行 auth --adb".into());
    }
    let package = String::from_utf8(adb.shell("dumpsys package com.ss.android.ugc.aweme", false)?)
        .map_err(|_| "应用版本编码无效")?;
    let metadata = adb.package_metadata()?;
    validate_package_metadata(&metadata, &package)?;
    for key in [
        "versionName",
        "versionCode",
        "manifestVersionCode",
        "updateVersionCode",
    ] {
        device[key] = metadata[key].clone();
    }
    device["identitySources"] = json!({
        "deviceId": "applog_stats.xml, corroborated by current Keva repository",
        "appVersions":"PackageManager SS_VERSION_CODE/SS_VERSION_NAME/UPDATE_VERSION_CODE and installed APK version, cross-checked against dumpsys"
    });
    let ua = text(&metadata, "userAgent")?;
    let mut session =
        json!({"schema":1,"source":"adb","device":device,"cookies":[],"userAgent":ua});
    import_transport_metadata(&metadata, &mut session)?;
    // Initialize the native SDK before taking either identity snapshot. Opening
    // the app later can refresh installation metadata after we already read it.
    if crate::native_auth::supported(&session) {
        adb.shell(
            "am start -W -n com.ss.android.ugc.aweme/com.ss.android.ugc.aweme.splash.SplashActivity",
            false,
        )?;
    }
    let prefs = parse_preferences(&read(&adb, "shared_prefs/applog_stats.xml")?)?;
    let device_id = prefs
        .get("device_id")
        .filter(|v| valid_id(v))
        .ok_or("应用缺少有效 device_id")?;
    let keva = read(&adb, "files/keva/repo/applog_stats/applog_stats.blk")?;
    // Newer apps migrate SharedPreferences to Keva. A legacy XML value alone
    // is insufficient: require corroboration in the current app repository.
    if !contains_id(&keva, device_id) {
        return Err("旧设备标识与当前应用记录不一致，拒绝导入".into());
    }
    session["device"]["deviceId"] = json!(device_id);
    if crate::native_auth::supported(&session) {
        let install_id = corroborated_install_id(&prefs, &keva)?;
        session["device"]["installId"] = json!(install_id);
        session["device"]["identitySources"]["installId"] =
            json!("applog_stats.xml, corroborated by current Keva repository");
        let (login, account) =
            crate::native_auth::import(&adb, &session)?.ok_or("当前版本未提供原生 Token 导入")?;
        session["account"] = account;
        session["nativeAuth"] = login;
        // Optional compatibility credentials must independently confirm this
        // same SDK account and installation. Failure never downgrades Token auth.
        if let Ok((cookies, source)) = snapshot_cookies(&adb) {
            let mut candidate = session.clone();
            candidate["cookies"] = json!(cookies);
            if cookie_install_id(&candidate).ok().flatten() == Some(install_id.to_owned())
                && verify(&candidate).is_ok_and(|account| account["id"] == session["account"]["id"])
            {
                session["cookies"] = candidate["cookies"].clone();
                session["nativeCookieSource"] = json!(source);
            }
        }
    } else {
        let (cookies, cookie_source) = snapshot_cookies(&adb)?;
        session["cookies"] = json!(cookies);
        let install_id = cookie_install_id(&session)?.ok_or("登录态缺少 install_id")?;
        if !contains_id(&keva, &install_id) {
            return Err("Cookie 的安装标识与当前应用记录不一致，拒绝导入".into());
        }
        session["device"]["installId"] = json!(install_id);
        session["device"]["identitySources"]["installId"] =
            json!("native-domain Cookie, corroborated by current Keva repository");
        session["nativeCookieSource"] = json!(cookie_source);
        // Passport registration-device metadata must not replace local identity.
        session["account"] = verify(&session)?;
    }
    session["verifiedAt"] = json!(now()?);
    session["device"]
        .as_object_mut()
        .ok_or("设备记录无效")?
        .remove("authenticated");
    let final_prefs = parse_preferences(&read(&adb, "shared_prefs/applog_stats.xml")?)?;
    let final_keva = read(&adb, "files/keva/repo/applog_stats/applog_stats.blk")?;
    validate_import_identity(&session, &final_prefs, &final_keva)?;
    crate::native_auth::confirm_import(&adb, &session)?;
    store::save(&session)?;
    Ok(summary(&session, true))
}

fn validate_import_identity(
    session: &Value,
    prefs: &BTreeMap<String, String>,
    keva: &[u8],
) -> Result<(), String> {
    for (field, preference) in [("deviceId", "device_id"), ("installId", "install_id")] {
        let expected = text(&session["device"], field)?;
        let must_match_xml = field == "deviceId" || session.get("nativeAuth").is_some();
        if !contains_id(keva, expected)
            || (must_match_xml && prefs.get(preference).map(String::as_str) != Some(expected))
        {
            return Err("导入期间应用设备或安装标识已变化，未保存新的登录配置，请重试".into());
        }
    }
    Ok(())
}

fn corroborated_install_id<'a>(
    prefs: &'a BTreeMap<String, String>,
    keva: &[u8],
) -> Result<&'a str, String> {
    let id = prefs
        .get("install_id")
        .filter(|v| valid_id(v))
        .ok_or("应用缺少有效 install_id，未导入原生 Token")?;
    if !contains_id(keva, id) {
        return Err("应用安装标识与当前 Keva 记录不一致，拒绝导入".into());
    }
    Ok(id)
}

fn cookie_install_id(session: &Value) -> Result<Option<String>, String> {
    let cookies = session["cookies"].as_array().ok_or("Cookie 配置无效")?;
    let header = cookie_header(
        cookies,
        "aweme.snssdk.com",
        "/passport/account/info/v2/",
        now()?,
    )?;
    Ok(header
        .split("; ")
        .find_map(|p| p.strip_prefix("install_id="))
        .filter(|v| valid_id(v))
        .map(str::to_owned))
}

pub fn status(offline: bool) -> Result<Value, String> {
    let session = store::load()?;
    validate_session(&session)?;
    if offline {
        return Ok(summary(&session, false));
    }
    if session.get("nativeAuth").is_some() {
        crate::native_auth::verify_online(&session)?;
        return Ok(summary(&session, true));
    }
    let account = verify(&session)?;
    if account["id"] != session["account"]["id"] {
        return Err("当前登录态账号与保存的账号不同，请重新运行 auth --adb".into());
    }
    Ok(summary(&session, true))
}

fn summary(session: &Value, verified: bool) -> Value {
    json!({"source":session["source"], "account":session["account"],
        "serial":session["device"]["serial"], "model":session["device"]["model"],
        "appVersion":session["device"]["versionName"],
        "deviceIdentityCorroborated":true,"onlineVerified":verified,
        "nativeTokenImported":session.get("nativeAuth").is_some(),
        "nativeCookiesImported":session["cookies"].as_array().is_some_and(|cookies| !cookies.is_empty()),
        "nativeTokenOnlineVerified":verified && session.get("nativeAuth").is_some(),
        "lastVerifiedAt":session["verifiedAt"]})
}

fn verify(session: &Value) -> Result<Value, String> {
    validate_session(session)?;
    let cookies = session["cookies"].as_array().ok_or("Cookie 配置无效")?;
    let header = cookie_header(
        cookies,
        "aweme.snssdk.com",
        "/passport/account/info/v2/",
        now()?,
    )?;
    let mut headers = reqwest::header::HeaderMap::new();
    let mut cookie =
        reqwest::header::HeaderValue::from_str(&header).map_err(|_| "账号 Cookie 请求头无效")?;
    cookie.set_sensitive(true);
    headers.insert("cookie", cookie);
    verify_account(session, headers, text(session, "userAgent")?)
}

fn verify_account(
    session: &Value,
    headers: reqwest::header::HeaderMap,
    agent: &str,
) -> Result<Value, String> {
    validate_session(session)?;
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "无法创建账号验证客户端")?;
    let params = request_identity(session)?;
    let response = client
        .get(ACCOUNT_URL)
        .query(&params)
        .headers(headers)
        .header("User-Agent", agent)
        .send()
        .map_err(|_| "账号验证请求失败，未保存新的登录配置")?;
    if !response.status().is_success() {
        return Err(format!("账号验证返回 HTTP {}", response.status().as_u16()));
    }
    let body: Value = response
        .json()
        .map_err(|_| "账号验证返回无法识别的内容，可能触发风控")?;
    parse_account(&body)
}

pub(crate) fn request_identity(session: &Value) -> Result<Vec<(&str, &str)>, String> {
    let device = &session["device"];
    Ok(vec![
        ("aid", "1128"),
        ("app_name", "aweme"),
        ("device_platform", "android"),
        ("device_id", text(device, "deviceId")?),
        ("iid", text(device, "installId")?),
        ("version_name", text(device, "versionName")?),
        ("version_code", text(device, "versionCode")?),
        (
            "manifest_version_code",
            text(device, "manifestVersionCode")?,
        ),
        ("update_version_code", text(device, "updateVersionCode")?),
        ("os", "android"),
        ("app_type", "normal"),
        ("device_type", text(device, "model")?),
        ("device_brand", text(device, "brand")?),
        ("os_version", text(device, "androidRelease")?),
        ("os_api", text(device, "sdk")?),
    ])
}

fn validate_package_metadata(metadata: &Value, dump: &str) -> Result<(), String> {
    let (name, manifest) = package_version(dump)?;
    if metadata["package"] != "com.ss.android.ugc.aweme"
        || metadata["versionName"] != name
        || metadata["manifestVersionCode"] != manifest
    {
        return Err("安装包元数据与当前应用版本不一致，未导入登录态".into());
    }
    for key in ["versionCode", "manifestVersionCode", "updateVersionCode"] {
        if !valid_id(text(metadata, key)?) {
            return Err("安装包缺少有效的 API、APK 或更新版本码".into());
        }
    }
    let ua = text(metadata, "userAgent")?;
    if ua.is_empty() || reqwest::header::HeaderValue::from_str(ua).is_err() {
        return Err("设备运行时 User-Agent 无效".into());
    }
    if let Some(value) = metadata.get("cronetDefaultUserAgent") {
        validate_default_user_agent(value, &metadata["manifestVersionCode"])?;
    }
    Ok(())
}

fn validate_default_user_agent(value: &Value, manifest: &Value) -> Result<(), String> {
    let ua = value.as_str().ok_or("SDK 默认 User-Agent 格式无效")?;
    let manifest = manifest
        .as_str()
        .ok_or("SDK 默认 User-Agent 缺少 APK 版本绑定")?;
    let prefix = format!("com.ss.android.ugc.aweme/{manifest} (Linux; U; Android ");
    if !valid_id(manifest)
        || ua.len() > 4096
        || !ua.starts_with(&prefix)
        || !ua.contains("; Cronet/TTNetVersion:")
        || !ua.ends_with(')')
        || reqwest::header::HeaderValue::from_str(ua).is_err()
    {
        return Err("SDK 默认 User-Agent 无效或与 APK 版本不一致".into());
    }
    Ok(())
}

fn import_transport_metadata(metadata: &Value, session: &mut Value) -> Result<(), String> {
    if let Some(ua) = metadata.get("cronetDefaultUserAgent") {
        validate_default_user_agent(ua, &session["device"]["manifestVersionCode"])?;
        session["nativeTransport"] = json!({
            "cronetDefaultUserAgent":ua,
            "userAgentSource":"installed-apk-cronet-default",
            "finalRequestVerified":false
        });
    }
    Ok(())
}

fn parse_account(body: &Value) -> Result<Value, String> {
    let data = &body["data"];
    if body["message"] != "success" || data["is_visitor_account"] == true {
        return Err("服务端未确认有效登录账号，请在手机上重新登录".into());
    }
    let app_id = data["app_id"]
        .as_u64()
        .map(|v| v.to_string())
        .or_else(|| data["app_id"].as_str().map(str::to_owned));
    let id = data["user_id_str"]
        .as_str()
        .filter(|v| valid_id(v))
        .map(str::to_owned)
        .or_else(|| {
            data["user_id"]
                .as_u64()
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
        })
        .ok_or("服务端未返回有效账号 ID")?;
    if app_id.as_deref() != Some("1128") {
        return Err("服务端返回了其他应用的账号，拒绝保存".into());
    }
    // Explicit allowlist: never print or persist the passport's phone, email,
    // session_key, or other account recovery material.
    Ok(
        json!({"id":id,"nickname":data["screen_name"].as_str().unwrap_or(""),
        "secUserId":data["sec_user_id"].as_str().unwrap_or("")}),
    )
}

pub(crate) fn validate_session(session: &Value) -> Result<(), String> {
    if session["schema"] != 1 || session["source"] != "adb" {
        return Err("不支持的设备登录配置格式".into());
    }
    for name in ["deviceId", "installId"] {
        if !valid_id(text(&session["device"], name)?) {
            return Err("设备登录配置中的标识无效".into());
        }
    }
    if let Some(transport) = session.get("nativeTransport") {
        if transport["userAgentSource"] != "installed-apk-cronet-default"
            || transport["finalRequestVerified"] != false
        {
            return Err("不支持的原生网络元数据格式".into());
        }
        validate_default_user_agent(
            &transport["cronetDefaultUserAgent"],
            &session["device"]["manifestVersionCode"],
        )?;
    }
    crate::native_auth::validate(session)
}

fn read(adb: &Adb, relative: &str) -> Result<Vec<u8>, String> {
    adb.shell(&format!("cat {DATA}{relative}"), true)
        .map_err(|_| format!("无法读取手机应用文件 {relative}，请检查解锁状态与 root 授权"))
}

fn select_cookie_files(found: &str) -> Result<Vec<&'static str>, String> {
    let mut selected = Vec::new();
    for line in found.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let candidate = COOKIE_FILES
            .iter()
            .copied()
            .find(|candidate| *candidate == line)
            .ok_or("应用 Cookie 数据库路径不在支持范围内")?;
        if selected.contains(&candidate) {
            return Err("应用 Cookie 数据库路径重复，未导入登录态".into());
        }
        selected.push(candidate);
    }
    if selected.is_empty() {
        return Err("没有可读取的受支持 Cookie 数据库，请确认手机解锁及 root 授权".into());
    }
    Ok(selected)
}

fn snapshot_cookies(adb: &Adb) -> Result<(Vec<Value>, String), String> {
    let candidates = COOKIE_FILES.join(" ");
    let found = adb.shell(
        &format!(
            "for path in {candidates}; do if [ -f {DATA}$path ]; then echo \"$path\"; fi; done"
        ),
        true,
    )?;
    let found = std::str::from_utf8(&found).map_err(|_| "应用 Cookie 数据库路径编码无效")?;
    let files = select_cookie_files(found)?;
    let mut snapshots = Vec::new();
    for file in &files {
        snapshots.push(snapshot_cookie_file(adb, file)?);
    }
    let cookies = shared_cookie_snapshot(&snapshots, now()?)?;
    Ok((cookies, files.join(", ")))
}

fn shared_cookie_snapshot(snapshots: &[Vec<Value>], timestamp: u64) -> Result<Vec<Value>, String> {
    let first = snapshots.first().ok_or("没有可读取的 Cookie 快照")?;
    if snapshots.len() == 1 {
        return Ok(first.clone());
    }
    let identity = |cookies: &[Value]| -> Result<BTreeMap<String, String>, String> {
        let header = cookie_header(
            cookies,
            "aweme.snssdk.com",
            "/passport/account/info/v2/",
            timestamp,
        )?;
        let fields: BTreeMap<_, _> = header
            .split("; ")
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                (name == "install_id"
                    || name.contains("session")
                    || name.starts_with("sid_")
                    || name.starts_with("uid_")
                    || name.starts_with("passport_"))
                .then(|| (name.to_owned(), value.to_owned()))
            })
            .collect();
        if !fields.contains_key("install_id")
            || !(fields.contains_key("sessionid") || fields.contains_key("sid_tt"))
        {
            return Err("多个 Cookie 数据库缺少可核对的设备和会话字段".into());
        }
        Ok(fields)
    };
    let expected = identity(first)?;
    for snapshot in &snapshots[1..] {
        if identity(snapshot)? != expected {
            return Err("多个 Cookie 数据库的设备或登录身份不一致，拒绝混合导入".into());
        }
    }
    // Every retained row must exist unchanged in every complete snapshot.
    // Never choose a conflicting device/tracking Cookie by filesystem priority.
    let common: Vec<_> = first
        .iter()
        .filter(|row| snapshots[1..].iter().all(|snapshot| snapshot.contains(row)))
        .cloned()
        .collect();
    if identity(&common)? != expected {
        return Err("多个 Cookie 数据库的身份字段有效期或属性不一致，未导入登录态".into());
    }
    Ok(common)
}

fn snapshot_cookie_file(adb: &Adb, cookie_file: &str) -> Result<Vec<Value>, String> {
    for _ in 0..3 {
        let db = read(adb, cookie_file)?;
        let wal = adb.shell(
            &format!("if [ -f {DATA}{cookie_file}-wal ]; then cat {DATA}{cookie_file}-wal; fi"),
            true,
        )?;
        let second_db = read(adb, cookie_file)?;
        let second_wal = adb.shell(
            &format!("if [ -f {DATA}{cookie_file}-wal ]; then cat {DATA}{cookie_file}-wal; fi"),
            true,
        )?;
        if db != second_db || wal != second_wal {
            continue;
        }
        let dir = tempfile::tempdir().map_err(|_| "无法创建私有登录态临时目录")?;
        let path = dir.path().join("Cookies");
        fs::write(&path, db).map_err(|_| "无法写入登录态快照")?;
        if !wal.is_empty() {
            fs::write(dir.path().join("Cookies-wal"), wal).map_err(|_| "无法写入登录态事务快照")?;
        }
        if let Ok(cookies) = read_cookies(&path) {
            return Ok(cookies);
        }
    }
    Err("无法读取稳定的 Cookie 数据库快照，请稍后重试".into())
}

fn read_cookies(path: &std::path::Path) -> Result<Vec<Value>, rusqlite::Error> {
    let db = Connection::open(path)?;
    let integrity: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut statement = db.prepare("SELECT host_key,name,value,path,expires_utc,is_secure,has_expires,length(encrypted_value) FROM cookies WHERE host_key IN ('.snssdk.com','aweme.snssdk.com','.amemv.com','.douyin.com','www.douyin.com')")?;
    statement
        .query_map([], |row| {
            Ok(
                json!({"domain":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,
            "value":row.get::<_,String>(2)?,"path":row.get::<_,String>(3)?,
            "expiresChrome":row.get::<_,i64>(4)?,"secure":row.get::<_,i64>(5)? != 0,
            "persistent":row.get::<_,i64>(6)? != 0,"encrypted":row.get::<_,i64>(7)? != 0}),
            )
        })?
        .collect()
}

pub(crate) fn cookie_header(
    cookies: &[Value],
    host: &str,
    path: &str,
    timestamp: u64,
) -> Result<String, String> {
    let mut selected: BTreeMap<&str, (&str, usize, usize)> = BTreeMap::new();
    let chrome_now = (i128::from(timestamp) + 11_644_473_600) * 1_000_000;
    for cookie in cookies {
        let domain = text(cookie, "domain")?;
        let matches = if let Some(domain) = domain.strip_prefix('.') {
            host == domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        } else {
            host == domain
        };
        let cookie_path = text(cookie, "path")?;
        let path_matches = path == cookie_path
            || path
                .strip_prefix(cookie_path)
                .is_some_and(|suffix| cookie_path.ends_with('/') || suffix.starts_with('/'));
        if !matches || !path_matches {
            continue;
        }
        if cookie["persistent"] == true
            && i128::from(
                cookie["expiresChrome"]
                    .as_i64()
                    .ok_or("Cookie 有效期无效")?,
            ) <= chrome_now
        {
            continue;
        }
        if cookie["encrypted"] == true {
            return Err("应用 Cookie 已加密，无法导入该登录态".into());
        }
        let name = text(cookie, "name")?;
        let value = text(cookie, "value")?;
        if value.is_empty() {
            continue;
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || value.bytes().any(|b| b < 0x21 || b == 0x7f || b == b';')
        {
            return Err("Cookie 含无效的请求头字符".into());
        }
        let priority = (cookie_path.len(), domain.len());
        match selected.get(name) {
            Some((old, plen, dlen)) if (*plen, *dlen) == priority && *old != value => {
                return Err("登录态存在同优先级的冲突 Cookie，拒绝混合账号".into());
            }
            Some((_, plen, dlen)) if (*plen, *dlen) >= priority => (),
            _ => {
                selected.insert(name, (value, priority.0, priority.1));
            }
        }
    }
    if !selected.contains_key("sessionid") {
        return Err("没有该应用域下有效的 sessionid".into());
    }
    Ok(selected
        .into_iter()
        .map(|(k, (v, _, _))| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; "))
}

fn parse_preferences(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let mut reader = Reader::from_reader(bytes);
    let mut values = BTreeMap::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|_| "应用设备配置不是有效 XML")?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e)
                if matches!(e.name().as_ref(), "string" | "long" | "int") =>
            {
                let mut name = None;
                let mut value = None;
                for attribute in e.attributes() {
                    let a = attribute.map_err(|_| "应用设备配置属性无效")?;
                    let v = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| "应用设备配置编码无效")?
                        .into_owned();
                    match a.key.as_ref() {
                        "name" => name = Some(v),
                        "value" => value = Some(v),
                        _ => (),
                    }
                }
                if let Some(name) =
                    name.filter(|n| matches!(n.as_str(), "device_id" | "install_id"))
                {
                    if matches!(event, Event::Start(_)) {
                        value = Some(
                            reader
                                .read_text(e.name())
                                .map_err(|_| "应用设备配置内容无效")?
                                .into_inner()
                                .into_owned(),
                        );
                    }
                    if values.insert(name, value.unwrap_or_default()).is_some() {
                        return Err("设备配置包含重复标识".into());
                    }
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }
    Ok(values)
}

fn package_version(output: &str) -> Result<(String, String), String> {
    let version = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("versionName="))
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        .ok_or("无法读取抖音应用版本")?;
    let code = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("versionCode="))
        .and_then(|line| line.split_whitespace().next())
        .filter(|v| valid_id(v))
        .ok_or("无法读取抖音应用版本号")?;
    Ok((version.to_owned(), code.to_owned()))
}

fn contains_id(bytes: &[u8], id: &str) -> bool {
    if !valid_id(id) {
        return false;
    }
    // Keva .blk string payloads have a little-endian tagged length immediately
    // before their contents. Allocator residue after the declared length may
    // itself contain digits: checking ASCII boundaries rejects valid IDs, while
    // unrestricted substring matching accepts prefixes of different IDs.
    let expected = 0x8100_0000_u32 | id.len() as u32;
    bytes.windows(id.len()).enumerate().any(|(offset, value)| {
        value == id.as_bytes()
            && offset
                .checked_sub(4)
                .and_then(|start| bytes.get(start..offset))
                .is_some_and(|header| header == expected.to_le_bytes())
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 24
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.bytes().any(|b| b != b'0')
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("配置缺少字符串字段 {key}"))
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "系统时间无效".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn import_rejects_rotated_xml_identity_even_when_old_keva_values_remain() {
        let session = json!({"device":{"deviceId":"1234","installId":"5678"},"nativeAuth":{}});
        let mut prefs = BTreeMap::from([
            ("device_id".into(), "1234".into()),
            ("install_id".into(), "5678".into()),
        ]);
        let keva = b"\x04\x00\x00\x811234\x04\x00\x00\x815678\x04\x00\x00\x819999";
        assert!(validate_import_identity(&session, &prefs, keva).is_ok());
        for key in ["device_id", "install_id"] {
            let previous = prefs.insert(key.into(), "9999".into()).unwrap();
            assert!(validate_import_identity(&session, &prefs, keva).is_err());
            prefs.insert(key.into(), previous);
        }
        assert!(validate_import_identity(&session, &prefs, b"").is_err());
    }
    #[test]
    fn native_install_id_requires_current_app_corroboration_without_cookie() {
        let prefs = std::collections::BTreeMap::from([("install_id".into(), "1234".into())]);
        assert_eq!(
            super::corroborated_install_id(&prefs, b"\x04\x00\x00\x811234").unwrap(),
            "1234"
        );
        assert!(super::corroborated_install_id(&prefs, b"\x04\x00\x00\x815678").is_err());
        assert!(super::corroborated_install_id(&prefs, b"\x08\x00\x00\x8112345678").is_err());
        assert!(
            super::corroborated_install_id(
                &std::collections::BTreeMap::new(),
                b"\x04\x00\x00\x811234"
            )
            .is_err()
        );
    }
    use super::*;
    #[test]
    fn cookie_database_discovery_accepts_supported_profiles_without_merging_them() {
        for candidate in COOKIE_FILES {
            assert_eq!(
                select_cookie_files(&format!("{candidate}\n")).unwrap(),
                vec![*candidate]
            );
        }
        for invalid in [
            "",
            "../private-profile/Cookies",
            "app_webview/Default/Cookies\napp_webview/Default/Cookies",
        ] {
            assert!(select_cookie_files(invalid).is_err());
        }
        assert_eq!(
            select_cookie_files(
                "app_webview/Default/Cookies\napp_webviewbytedance_com.ss.android.ugc.aweme/Cookies"
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn native_token_summary_never_exposes_imported_credentials() {
        let session = json!({"account":{"id":"123"},"nativeAuth":{
            "token":"private-fixture-token", "userAgent":"private-fixture-agent"}});
        let offline = summary(&session, false);
        assert_eq!(offline["nativeTokenImported"], true);
        assert_eq!(offline["nativeTokenOnlineVerified"], false);
        assert!(!offline.to_string().contains("private-fixture"));
        assert!(offline.get("nativeAuth").is_none());
        assert_eq!(summary(&session, true)["nativeTokenOnlineVerified"], true);
    }

    #[test]
    fn sdk_default_agent_stays_build_bound_private_and_separate_from_effective_agent() {
        let ua = "com.ss.android.ugc.aweme/400801 (Linux; U; Android 16; fixture; Cronet/TTNetVersion:private-fixture)";
        let metadata = json!({"cronetDefaultUserAgent":ua});
        let mut session = json!({"schema":1,"source":"adb","userAgent":"Dalvik/fixture",
            "device":{"deviceId":"111","installId":"222","manifestVersionCode":"400801"}});
        import_transport_metadata(&metadata, &mut session).unwrap();
        validate_session(&session).unwrap();
        assert_eq!(session["nativeTransport"]["cronetDefaultUserAgent"], ua);
        assert_eq!(session["nativeTransport"]["finalRequestVerified"], false);
        assert_eq!(session["userAgent"], "Dalvik/fixture");
        assert!(
            !summary(&session, false)
                .to_string()
                .contains("private-fixture")
        );
        for invalid in [
            json!(ua.replace("400801", "400800")),
            json!(format!("{ua}\r\nprivate-fixture")),
            json!("Dalvik/private-fixture"),
            Value::Null,
            json!(123),
        ] {
            let mut bad = session.clone();
            bad["nativeTransport"]["cronetDefaultUserAgent"] = invalid;
            let error = validate_session(&bad).unwrap_err();
            assert!(!error.contains("private-fixture"));
        }
        let mut bad = session.clone();
        bad["nativeTransport"]["finalRequestVerified"] = json!(true);
        assert!(validate_session(&bad).is_err());
        session.as_object_mut().unwrap().remove("nativeTransport");
        import_transport_metadata(&json!({}), &mut session).unwrap();
        assert!(session.get("nativeTransport").is_none());
        validate_session(&session).unwrap();
    }

    #[test]
    fn distinguishes_api_apk_and_update_versions_without_guessing() {
        let metadata = json!({"package":"com.ss.android.ugc.aweme","versionName":"40.8.0","versionCode":"400800","manifestVersionCode":"400801","updateVersionCode":"40809900","userAgent":"Dalvik/fixture"});
        let dump = "versionCode=400801 minSdk=26 targetSdk=36\nversionName=40.8.0";
        validate_package_metadata(&metadata, dump).unwrap();
        let mut device = metadata.clone();
        for (key, value) in [
            ("deviceId", "111"),
            ("installId", "222"),
            ("model", "fixture"),
            ("brand", "fixture"),
            ("androidRelease", "16"),
            ("sdk", "36"),
        ] {
            device[key] = json!(value);
        }
        let session = json!({"device":device});
        let params = request_identity(&session)
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(params["version_code"], "400800");
        assert_eq!(params["manifest_version_code"], "400801");
        assert_eq!(params["update_version_code"], "40809900");
        for (key, value) in [
            ("package", "other.app"),
            ("versionName", "40.9.0"),
            ("manifestVersionCode", "400899"),
            ("versionCode", "0"),
            ("updateVersionCode", "invalid"),
            ("userAgent", "invalid\r\nInjected: header"),
        ] {
            let mut bad = metadata.clone();
            bad[key] = json!(value);
            assert!(validate_package_metadata(&bad, dump).is_err(), "{key}");
        }
    }
    fn cookie(domain: &str, name: &str, value: &str) -> Value {
        json!({"domain":domain,"name":name,"value":value,"path":"/","persistent":false,"encrypted":false})
    }
    #[test]
    fn cookie_profiles_require_identical_identity_and_keep_only_shared_records() {
        let common = vec![
            cookie(".snssdk.com", "install_id", "222"),
            cookie(".snssdk.com", "sessionid", "fixture-session"),
            cookie(".snssdk.com", "uid_tt", "fixture-uid"),
        ];
        let mut a = common.clone();
        a.push(cookie(".snssdk.com", "odin_tt", "context-a"));
        let mut b = common.clone();
        b.push(cookie(".snssdk.com", "odin_tt", "context-b"));
        assert_eq!(
            shared_cookie_snapshot(&[a.clone(), b.clone()], 100).unwrap(),
            common
        );
        for index in 0..3 {
            let mut conflict = b.clone();
            conflict[index]["value"] = json!("private-other-identity");
            let error = shared_cookie_snapshot(&[a.clone(), conflict], 100).unwrap_err();
            assert!(!error.contains("private-other-identity"));
        }
        b[1]["secure"] = json!(true);
        assert!(shared_cookie_snapshot(&[a, b], 100).is_err());
    }

    #[test]
    fn isolates_domains_and_expired_sessions() {
        let mut expired = cookie(".snssdk.com", "sessionid", "expired");
        expired["persistent"] = json!(true);
        expired["expiresChrome"] = json!(1);
        let rows = vec![
            cookie(".evil-snssdk.com", "sessionid", "evil"),
            cookie(".amemv.com", "sessionid", "other"),
            expired,
        ];
        assert!(cookie_header(&rows, "aweme.snssdk.com", "/", 1).is_err());
    }
    #[test]
    fn rejects_conflicting_sessions_and_header_injection() {
        assert!(
            cookie_header(
                &[
                    cookie(".snssdk.com", "sessionid", "a"),
                    cookie(".snssdk.com", "sessionid", "b")
                ],
                "aweme.snssdk.com",
                "/",
                1
            )
            .is_err()
        );
        assert!(
            cookie_header(
                &[cookie(".snssdk.com", "sessionid", "a\r\nX: a")],
                "aweme.snssdk.com",
                "/",
                1
            )
            .is_err()
        );
    }
    #[test]
    fn path_boundaries_and_host_specific_cookie_take_precedence() {
        let mut bad_path = cookie("aweme.snssdk.com", "sessionid", "wrong-path");
        bad_path["path"] = json!("/account");
        let rows = vec![
            cookie(".snssdk.com", "sessionid", "parent"),
            cookie("aweme.snssdk.com", "sessionid", "host"),
            bad_path,
        ];
        assert_eq!(
            cookie_header(&rows, "aweme.snssdk.com", "/accounts", 1).unwrap(),
            "sessionid=host"
        );
    }
    #[test]
    fn accepts_both_android_preference_representations() {
        let p=parse_preferences(br#"<map><string name="device_id">1234</string><long name="install_id" value="5678"/></map>"#).unwrap();
        assert_eq!(p["device_id"], "1234");
        assert_eq!(p["install_id"], "5678");
        assert!(parse_preferences(br#"<map><string name="device_id">1</string><string name="device_id">2</string></map>"#).is_err());
    }
    #[test]
    fn anonymous_success_and_other_app_are_not_login_proof() {
        assert!(parse_account(&json!({"message":"success","data":{}})).is_err());
        assert!(
            parse_account(&json!({"message":"success","data":{"app_id":123,"user_id":456}}))
                .is_err()
        );
        let account=parse_account(&json!({"message":"success","data":{"app_id":1128,"user_id_str":"456","screen_name":"test","mobile":"private","session_key":"secret"}})).unwrap();
        assert_eq!(account["id"], "456");
        assert!(account.get("mobile").is_none());
        assert!(account.get("session_key").is_none());
    }
    #[test]
    fn corroboration_does_not_accept_substrings_of_other_ids() {
        assert!(!contains_id(b"012345", "1234"));
        assert!(contains_id(b"\x04\x00\x00\x8112345678", "1234"));
        assert!(!contains_id(b"\x08\x00\x00\x8112345678", "1234"));
        assert!(!contains_id(b"\x04\x00\x00\x011234", "1234"));
        assert!(!contains_id(b"", ""));
    }
}
