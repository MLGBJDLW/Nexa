//! A private per-installation CA; only its public certificate is exported.
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};
pub struct LanIdentity {
    pub config: axum_server::tls_rustls::RustlsConfig,
    pub certificate_path: PathBuf,
}
pub fn lan_addresses() -> Result<Vec<IpAddr>, String> {
    let mut addresses=if_addrs::get_if_addrs().map_err(|e|e.to_string())?.into_iter().map(|entry|entry.ip()).filter(|ip|matches!(ip,IpAddr::V4(ip) if ip.is_private() && !ip.is_loopback() && !ip.is_link_local())).collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    Ok(addresses)
}
pub async fn identity(directory: &Path, addresses: &[IpAddr]) -> Result<LanIdentity, String> {
    let directory = directory.to_owned();
    let addresses = addresses.to_vec();
    let (pem, key, certificate_path) =
        tokio::task::spawn_blocking(move || prepare(&directory, &addresses))
            .await
            .map_err(|e| e.to_string())??;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config =
        axum_server::tls_rustls::RustlsConfig::from_pem(pem.into_bytes(), key.into_bytes())
            .await
            .map_err(|e| e.to_string())?;
    Ok(LanIdentity {
        config,
        certificate_path,
    })
}
fn prepare(directory: &Path, addresses: &[IpAddr]) -> Result<(String, String, PathBuf), String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let certificate_path = directory.join("Nexa-LAN-CA.crt");
    let key_path = directory.join("lan-ca-key.pem");
    let (ca_pem, key) = if certificate_path.exists() || key_path.exists() {
        (
            std::fs::read_to_string(&certificate_path).map_err(|e| e.to_string())?,
            KeyPair::from_pem(&std::fs::read_to_string(&key_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?,
        )
    } else {
        let key = KeyPair::generate().map_err(|e| e.to_string())?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, "Nexa private LAN certificate authority");
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(3650);
        let pem = params.self_signed(&key).map_err(|e| e.to_string())?.pem();
        write_private(&key_path, key.serialize_pem().as_bytes())?;
        std::fs::write(&certificate_path, &pem).map_err(|e| e.to_string())?;
        (pem, key)
    };
    let issuer = Issuer::from_ca_cert_pem(&ca_pem, key).map_err(|e| e.to_string())?;
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut params = CertificateParams::new(
        addresses
            .iter()
            .map(ToString::to_string)
            .chain(["localhost".into(), "127.0.0.1".into()])
            .collect::<Vec<_>>(),
    )
    .map_err(|e| e.to_string())?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Nexa LAN");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(90);
    let certificate = params.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    Ok((
        format!("{}{}", certificate.pem(), ca_pem),
        key.serialize_pem(),
        certificate_path,
    ))
}
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
