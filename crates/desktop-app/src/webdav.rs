//! 极简 WebDAV 客户端（阶段 1）：只实现同步需要的 PROPFIND/GET/PUT/HEAD/MKCOL。
//!
//! 边界：
//! - 只允许 HTTPS（拒绝 `http://`，避免凭证明文传输）；
//! - 只访问用户配置的同步根目录；不遍历父目录；
//! - 不解析正文，只搬运同步包文件；
//! - 失败一律带可读原因，不静默降级为空目录。

use std::io::Read;
use std::time::Duration;

use base64::Engine as _;

/// WebDAV 配置（来自宿主设置/环境变量；密码不写日志）。
#[derive(Clone)]
pub struct WebDavConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for WebDavConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavConfig")
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl WebDavConfig {
    /// 从环境变量读取（保留给脚本/测试；正常入口是设置页配置）。
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("NR_WEBDAV_URL").ok()?.trim().to_string();
        if base_url.is_empty() {
            return None;
        }
        let username = std::env::var("NR_WEBDAV_USER").unwrap_or_default();
        let password = std::env::var("NR_WEBDAV_PASSWORD").unwrap_or_default();
        Some(Self {
            base_url,
            username,
            password,
        })
    }

    /// 配置是否完整（URL 必须是 HTTPS）。
    pub fn validate(&self) -> Result<(), String> {
        let url = self.base_url.trim();
        #[cfg(test)]
        let loopback_test = url.starts_with("http://127.0.0.1:");
        #[cfg(not(test))]
        let loopback_test = false;
        if !url.starts_with("https://") && !loopback_test {
            return Err("WebDAV 地址必须以 https:// 开头（拒绝明文 HTTP）".to_string());
        }
        if self.username.trim().is_empty() {
            return Err("WebDAV 用户名不能为空".to_string());
        }
        Ok(())
    }
}

/// WebDAV 客户端。
pub struct WebDavClient {
    agent: ureq::Agent,
    base: String,
    authorization: Option<String>,
}

impl WebDavClient {
    pub fn new(config: &WebDavConfig) -> Result<Self, String> {
        config.validate()?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .build();
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", config.username, config.password));
        Ok(Self {
            agent,
            // 统一以 `/` 结尾：避免 MKCOL 先 301 到带斜杠路径时丢失 Authorization。
            base: format!("{}/", config.base_url.trim().trim_end_matches('/')),
            authorization: Some(format!("Basic {token}")),
        })
    }

    fn url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            self.base.clone()
        } else {
            format!("{}{}", self.base, path)
        }
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let mut request = self.agent.request(method, &self.url(path));
        if let Some(auth) = &self.authorization {
            request = request.set("Authorization", auth);
        }
        request
    }

    fn status_error(method: &str, path: &str, error: ureq::Error) -> String {
        match error {
            ureq::Error::Status(401, _) => format!(
                "WebDAV {method} {} 认证失败（HTTP 401）：请确认使用 WebDAV 应用密码（不是登录密码）、用户名/邮箱正确，且地址是该服务的 WebDAV 根地址（例如坚果云 https://dav.jianguoyun.com/dav/）",
                if path.is_empty() { "/" } else { path }
            ),
            ureq::Error::Status(403, _) => format!(
                "WebDAV {method} {path} 被拒（HTTP 403）：目标目录可能只读，或账号没有在该目录创建文件的权限"
            ),
            ureq::Error::Status(405, _) => format!(
                "WebDAV {method} {path} 被拒（HTTP 405）：该服务器不允许此方法/路径（PUT 状态文件建议放在 states/ 子目录）"
            ),
            ureq::Error::Status(409, _) => format!(
                "WebDAV {method} {path} 冲突（HTTP 409）：父目录不存在或目标名称冲突"
            ),
            ureq::Error::Status(413, _) => format!(
                "WebDAV {method} {path} 失败（HTTP 413）：状态文件超过服务器单次上传限制"
            ),
            ureq::Error::Status(429, _) => format!(
                "WebDAV {method} {path} 被限流（HTTP 429）：请求过多，请等待后重试（坚果云免费版约 600 请求/30 分钟）"
            ),
            ureq::Error::Status(507, _) => format!(
                "WebDAV {method} {path} 失败（HTTP 507）：网盘配额不足"
            ),
            ureq::Error::Status(code, response) => {
                let text = response.into_string().unwrap_or_default();
                let text = text.trim();
                let shown_path = if path.is_empty() { "/" } else { path };
                if text.is_empty() {
                    format!("WebDAV {method} {shown_path} 失败：HTTP {code}")
                } else {
                    format!("WebDAV {method} {shown_path} 失败：HTTP {code}（{text}）")
                }
            }
            ureq::Error::Transport(transport) => {
                format!("WebDAV {method} {path} 网络错误：{transport}")
            }
        }
    }

    /// 创建集合（已存在视为成功；部分服务器对根返回 405）。
    pub fn ensure_dir(&self, path: &str) -> Result<(), String> {
        let response = self.request("MKCOL", path).call();
        match response {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(405, _)) | Err(ureq::Error::Status(409, _)) => Ok(()),
            Err(error) => Err(Self::status_error("MKCOL", path, error)),
        }
    }

    /// 轻量连通/认证检查：PROPFIND Depth 0。
    pub fn check_connection(&self) -> Result<(), String> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;
        self.request("PROPFIND", "")
            .set("Depth", "0")
            .set("Content-Type", "application/xml; charset=utf-8")
            .send_string(body)
            .map(|_| ())
            .map_err(|error| Self::status_error("PROPFIND", "", error))
    }

    /// 列出目录下一层的文件名（已做百分号解码）。
    pub fn list_names(&self, path: &str) -> Result<Vec<String>, String> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;
        let response = self
            .request("PROPFIND", path)
            .set("Depth", "1")
            .set("Content-Type", "application/xml; charset=utf-8")
            .send_string(body)
            .map_err(|error| Self::status_error("PROPFIND", path, error))?;
        let xml = response
            .into_string()
            .map_err(|error| format!("读取 WebDAV 目录失败：{error}"))?;
        let mut names = Vec::new();
        for href in extract_hrefs(&xml) {
            let decoded = percent_decode(&href);
            let trimmed = decoded.trim_end_matches('/');
            let Some(name) = trimmed.rsplit('/').next() else {
                continue;
            };
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            names.push(name.to_string());
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// 下载文件。
    pub fn get(&self, path: &str) -> Result<Vec<u8>, String> {
        let response = self
            .request("GET", path)
            .call()
            .map_err(|error| Self::status_error("GET", path, error))?;
        let mut reader = response.into_reader();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("读取 WebDAV 文件失败：{error}"))?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(bytes)
    }

    /// 上传文件（覆盖）。
    pub fn put(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.request("PUT", path)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(bytes)
            .map(|_| ())
            .map_err(|error| Self::status_error("PUT", path, error))
    }

    /// 文件是否存在（HEAD；部分服务器不支持 HEAD 时退化为 PROPFIND）。
    pub fn exists(&self, path: &str) -> Result<bool, String> {
        match self.request("HEAD", path).call() {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(ureq::Error::Status(405, _)) => {
                let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
                let name = path.rsplit('/').next().unwrap_or(path);
                Ok(self.list_names(parent)?.iter().any(|item| item == name))
            }
            Err(error) => Err(Self::status_error("HEAD", path, error)),
        }
    }
}

fn extract_hrefs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = xml.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<d:href>") {
        let start = cursor + start + "<d:href>".len();
        let Some(end_rel) = lower[start..].find("</d:href>") else {
            break;
        };
        let end = start + end_rel;
        out.push(xml[start..end].trim().to_string());
        cursor = end + "</d:href>".len();
    }
    // 兼容不带命名空间前缀的服务器。
    if out.is_empty() {
        let mut cursor = 0;
        while let Some(start) = lower[cursor..].find("<href>") {
            let start = cursor + start + "<href>".len();
            let Some(end_rel) = lower[start..].find("</href>") else {
                break;
            };
            let end = start + end_rel;
            out.push(xml[start..end].trim().to_string());
            cursor = end + "</href>".len();
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_plain_http_and_empty_user() {
        let config = WebDavConfig {
            base_url: "http://example.com/dav".to_string(),
            username: "u".to_string(),
            password: "p".to_string(),
        };
        assert!(config.validate().is_err());
        let config = WebDavConfig {
            base_url: "https://example.com/dav".to_string(),
            username: " ".to_string(),
            password: "p".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn parses_and_decodes_hrefs() {
        let xml = r#"<d:multistatus><d:response><d:href>/dav/state-a.json</d:href></d:response><d:response><d:href>/dav/books/%E4%B8%AD.txt</d:href></d:response></d:multistatus>"#;
        let names = extract_hrefs(xml);
        assert_eq!(names.len(), 2);
        assert_eq!(percent_decode(&names[1]).rsplit('/').next(), Some("中.txt"));
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    fn serve(mut stream: TcpStream, state: Arc<Mutex<BTreeMap<String, Vec<u8>>>>) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut request_line = String::new();
        reader.read_line(&mut request_line).expect("request line");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("/").to_string();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("body");

        let key = path
            .trim_start_matches("/dav")
            .trim_start_matches('/')
            .to_string();
        let (code, payload) = match method.as_str() {
            "MKCOL" => {
                if !key.is_empty() {
                    state.lock().unwrap().entry(key).or_default();
                }
                (201, Vec::new())
            }
            "PUT" => {
                state.lock().unwrap().insert(key, body);
                (201, Vec::new())
            }
            "HEAD" => {
                if state.lock().unwrap().contains_key(&key) {
                    (200, Vec::new())
                } else {
                    (404, Vec::new())
                }
            }
            "GET" => match state.lock().unwrap().get(&key).cloned() {
                Some(value) => (200, value),
                None => (404, Vec::new()),
            },
            "PROPFIND" => {
                let mut xml =
                    String::from(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">"#);
                for name in state.lock().unwrap().keys() {
                    if !name.contains('/') {
                        xml.push_str(&format!(
                            "<d:response><d:href>/{name}</d:href></d:response>"
                        ));
                    }
                }
                xml.push_str("</d:multistatus>");
                (207, xml.into_bytes())
            }
            _ => (405, Vec::new()),
        };
        let reason = if code == 200 || code == 201 || code == 207 {
            "OK"
        } else {
            "Not Found"
        };
        let head = format!(
            "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        stream.write_all(head.as_bytes()).expect("head");
        if method != "HEAD" {
            stream.write_all(&payload).expect("payload");
        }
    }

    #[test]
    fn speaks_minimal_webdav_over_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let state = Arc::new(Mutex::new(BTreeMap::new()));
        let server_state = Arc::clone(&state);
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(8) {
                if let Ok(stream) = stream {
                    serve(stream, Arc::clone(&server_state));
                }
            }
        });

        let config = WebDavConfig {
            base_url: format!("http://127.0.0.1:{port}/dav"),
            username: "u".to_string(),
            password: "p".to_string(),
        };
        let client = WebDavClient::new(&config).expect("client");
        client.ensure_dir("").expect("mkcol root");
        client.ensure_dir("books").expect("mkcol books");
        client.put("books/a.txt", b"hello").expect("put");
        assert!(client.exists("books/a.txt").expect("head"));
        assert_eq!(client.get("books/a.txt").expect("get"), b"hello");
        let names = client.list_names("").expect("propfind");
        assert!(names.iter().any(|name| name == "books"));

        drop(client);
        // 服务器线程是无限 accept 的测试桩；不要 join（否则会等第 8 个连接）。
        let _ = server;
    }
}
