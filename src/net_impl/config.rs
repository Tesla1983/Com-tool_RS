//! 网络通道（TCP 客户端 / UDP）的配置。
//!
//! 与 `serial_impl::config::SerialConfig` 同形：一个可序列化的参数结构体，
//! 由 UI 侧填好后经 `ByteChannel::configure` 提交（trait 用 `&dyn Any` 传递，
//! 实现方自己向下转型——见 `communicationtrait` 的说明）。

use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

/// 目标地址的默认值。`127.0.0.1` 而不是 `localhost`：**不经过 DNS**，
/// 既快又不受 hosts 文件影响，与"网络调试工具"的用法相符。
pub const DEFAULT_REMOTE_HOST: &str = "127.0.0.1";

/// 目标端口的默认值。取 1234 是因为它是各类 `nc -l` 示例里最常见的端口。
pub const DEFAULT_REMOTE_PORT: u16 = 1234;

/// 网络连接参数。
///
/// 只有一份配置给 TCP 与 UDP 共用：两者的差别全在"怎么用"（连 vs 绑定），
/// 而"连到哪"是同一件事。分开成两个结构体只会让 UI 侧多一个分支。
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct NetConfig {
    /// 目标主机（IP 字面量或域名）。
    pub remote_host: String,
    /// 目标端口。
    pub remote_port: u16,
    /// UDP 的本地绑定端口；`0` = 让系统分配（TCP 不用这一项）。
    ///
    /// 单独给一个字段而不是"绑定到目标端口"：UDP 调试经常需要**固定本地端口**
    /// 才能收到设备回包（对端可能只往固定端口发），但本地端口与目标端口没有
    /// 必然关系。
    pub local_port: u16,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            remote_host: DEFAULT_REMOTE_HOST.to_string(),
            remote_port: DEFAULT_REMOTE_PORT,
            local_port: 0,
        }
    }
}

impl NetConfig {
    /// 参数自检。**在建立连接之前调用**，把"地址空着 / 端口为 0"这类错误
    /// 变成一句能看懂的提示，而不是留给 `connect` 报一个 errno。
    pub fn validate(&self) -> io::Result<()> {
        if self.remote_host.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "目标地址不能为空",
            ));
        }
        if self.remote_port == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "目标端口不能为 0",
            ));
        }
        Ok(())
    }

    /// 把 `remote_host:remote_port` 解析成 socket 地址。
    ///
    /// 主机可以是 IP 字面量或域名；域名会**同步**走一次 DNS（可能阻塞几十毫秒到
    /// 数秒），这与"UI 线程不做阻塞 IO"的约定有张力——见 [`crate::net_impl::tcp`]
    /// 模块文档里对这次取舍的说明。解析出多个地址时取第一个（`ToSocketAddrs`
    /// 在 Windows 上按系统顺序返回）。
    pub fn remote_addr(&self) -> io::Result<SocketAddr> {
        self.validate()?;
        (self.remote_host.as_str(), self.remote_port)
            .to_socket_addrs()
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("无法解析目标地址 {}: {e}", self.remote_host),
                )
            })?
            .next()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "目标地址 {} 没有解析出任何 IP（端口 {}）",
                        self.remote_host, self.remote_port
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_loopback_and_non_zero() {
        let cfg = NetConfig::default();
        assert_eq!(cfg.remote_host, DEFAULT_REMOTE_HOST);
        assert_eq!(cfg.remote_port, DEFAULT_REMOTE_PORT);
        assert_eq!(cfg.local_port, 0, "本地端口默认应由系统分配");
        cfg.validate().expect("默认值必须通过自检");
    }

    /// 空地址 / 零端口必须被自检拦下——否则要等到 `connect` 才报一个看不懂的 errno。
    #[test]
    fn validate_rejects_empty_host_and_zero_port() {
        let bad_host = NetConfig {
            remote_host: "   ".to_string(),
            ..NetConfig::default()
        };
        let err = bad_host.validate().expect_err("空地址必须被拒绝");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("目标地址"),
            "错误信息应指出哪一项错了"
        );

        let bad_port = NetConfig {
            remote_port: 0,
            ..NetConfig::default()
        };
        let err = bad_port.validate().expect_err("零端口必须被拒绝");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("端口"), "错误信息应指出哪一项错了");

        // `remote_addr()` 也走同一条自检，不能绕过
        assert!(bad_host.remote_addr().is_err());
        assert!(bad_port.remote_addr().is_err());
    }

    /// IP 字面量必须能解析，且**不经过 DNS**（用 127.0.0.1 保证结果确定）。
    #[test]
    fn ip_literal_resolves_to_that_address() {
        let cfg = NetConfig {
            remote_host: "127.0.0.1".to_string(),
            remote_port: 4321,
            ..NetConfig::default()
        };
        let addr = cfg.remote_addr().expect("IP 字面量必须能解析");
        assert_eq!(addr.port(), 4321);
        assert!(addr.ip().is_loopback());
    }
}
