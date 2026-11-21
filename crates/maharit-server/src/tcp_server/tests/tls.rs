//! TLS 接続と証明書の再読み込み。

use super::*;

#[tokio::test]
async fn tls_server_serves_queries_and_rejects_plaintext() {
    use crate::tls::test_support::*;
    let cert = TempPem::new("srv_cert", TEST_SERVER_CERT_PEM);
    let key = TempPem::new("srv_key", TEST_SERVER_KEY_PEM);
    let (addr, _) = start_tls_server(cert.path(), key.path()).await;

    let mut stream = tls_connect(addr, cert.path()).await.unwrap();
    assert!(matches!(
        send_recv(&mut stream, &Request::Ping).await,
        Response::Pong
    ));
    let create = Request::Query {
        query: "CREATE (:T {v: 1})".to_string(),
        params: HashMap::new(),
        tx_id: None,
        session_token: None,
        row_format: Default::default(),
    };
    assert!(matches!(
        send_recv(&mut stream, &create).await,
        Response::Result { .. }
    ));

    // A plaintext client gets no protocol response: the server treats its
    // length prefix as a broken TLS record and closes the connection.
    let mut plain = TcpStream::connect(addr).await.unwrap();
    let json = serde_json::to_vec(&Request::Ping).unwrap();
    plain
        .write_all(&(json.len() as u32).to_be_bytes())
        .await
        .unwrap();
    plain.write_all(&json).await.unwrap();
    let mut buf = Vec::new();
    let read = timeout(Duration::from_secs(5), plain.read_to_end(&mut buf)).await;
    let got_pong = matches!(read, Ok(Ok(_)))
        && buf.len() > 4
        && serde_json::from_slice::<Response>(&buf[4..]).is_ok_and(|r| matches!(r, Response::Pong));
    assert!(!got_pong, "plaintext request was answered over a TLS port");

    // A client that does not trust the certificate cannot connect.
    let other_ca = TempPem::new("other_ca", TEST_SERVER_CERT2_PEM);
    assert!(tls_connect(addr, other_ca.path()).await.is_err());
}

#[tokio::test]
async fn tls_server_works_with_rust_client() {
    use crate::tls::test_support::*;
    use maharit_client::{Client, TlsClientConfig};
    let cert = TempPem::new("cli_cert", TEST_SERVER_CERT_PEM);
    let key = TempPem::new("cli_key", TEST_SERVER_KEY_PEM);
    let (addr, _) = start_tls_server(cert.path(), key.path()).await;

    let tls = TlsClientConfig::new("localhost").with_ca_cert(cert.path());
    let mut client = Client::connect_tls(&addr.to_string(), tls).await.unwrap();
    client.execute("CREATE (:P {name: 'tls'})").await.unwrap();
    let result = client.query("MATCH (p:P) RETURN p.name").await.unwrap();
    assert_eq!(result.row_count(), 1);
}

#[tokio::test]
async fn tls_certificate_reload_applies_to_new_connections() {
    use crate::tls::test_support::*;
    let cert = TempPem::new("reload_cert", TEST_SERVER_CERT_PEM);
    let key = TempPem::new("reload_key", TEST_SERVER_KEY_PEM);
    let ca1 = TempPem::new("reload_ca1", TEST_SERVER_CERT_PEM);
    let ca2 = TempPem::new("reload_ca2", TEST_SERVER_CERT2_PEM);
    let (addr, tls) = start_tls_server(cert.path(), key.path()).await;

    let mut old_conn = tls_connect(addr, ca1.path()).await.unwrap();
    assert!(tls_connect(addr, ca2.path()).await.is_err());

    // A broken certificate file keeps the previous certificate in use.
    std::fs::write(cert.path(), "not a certificate").unwrap();
    assert!(tls.reload().is_err());
    assert!(tls_connect(addr, ca1.path()).await.is_ok());

    // Watch the files and swap in the second certificate.
    let stop = Arc::new(AtomicBool::new(false));
    std::fs::write(cert.path(), TEST_SERVER_CERT_PEM).unwrap();
    let watcher = tls.spawn_watcher(Duration::from_millis(50), Arc::clone(&stop));
    tokio::time::sleep(Duration::from_millis(20)).await;
    std::fs::write(cert.path(), TEST_SERVER_CERT2_PEM).unwrap();
    std::fs::write(key.path(), TEST_SERVER_KEY2_PEM).unwrap();

    let mut switched = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if tls_connect(addr, ca2.path()).await.is_ok() {
            switched = true;
            break;
        }
    }
    stop.store(true, Ordering::SeqCst);
    watcher.await.unwrap();
    assert!(switched, "the reloaded certificate was not served");
    assert!(tls_connect(addr, ca1.path()).await.is_err());

    // A connection made before the reload keeps working.
    assert!(matches!(
        send_recv(&mut old_conn, &Request::Ping).await,
        Response::Pong
    ));
}

// ── Follower writes, snapshot sync and promotion (bug/152) ──────────────
