# Research: `flo-engine` Client Auth Mechanism

- **Scope:** Client authentication for the `flo-engine` cloud rule-server.
- **Transport:** Zenoh 1.9.0 (`features = ["unstable"]`), deployed as a router + rule service; robots are containers on physical robots acting as Zenoh clients/peers.
- **Status:** No authentication exists today. `src/transport.rs` opens `zenoh::Config::default()` with no TLS/ACL config; `src/rules.rs` defines key-expressions (`robot/{id}/client/liveliness`, `robot/{id}/local/rules`, `robot/{id}/signal/**`, `stop/**`, `lidar/**`) with no subject binding.
- **Repo constraints:** `flo-rs`, edition 2024, `#![forbid(unsafe_code)]`. AGENTS.md requires admin approval for any new dependency; no crypto crate is pulled directly today.

---

## 1. Field survey: functional-safety field-robotics auth

### 1.1 ROS 2 DDS-Security / SROS2 (reference baseline)
ROS 2's Secure ROS 2 (SROS2) layers DDS-Security over the DDS middleware:
- **Identity:** X.509 v3 certificates signed by an Identity CA; each enclave (process/participant) gets a unique keypair + cert [ROS 2 DDS-Security design](https://design.ros2.org/articles/ros2_dds_security.html).
- **Permissions:** a Permissions CA signs a `permissions.xml` (governance + access rules); the subject name must match the identity cert [OpenDDS DDS Security](https://opendds.readthedocs.io/en/latest-release/devguide/dds_security.html).
- **Governance:** `governance.xml` enforces `allow_unauthenticated_participants=false`, encryption of discovery/liveliness/RTPS [Fast-DDS issue #5707 example](https://github.com/eProsima/Fast-DDS/issues/5707).
- **Operational lesson:** SROS2 tooling creates a keystore tree per enclave; a known pitfall is a *single* CA used as both Identity and Permissions CA (cert-chain-of-trust hole, [ros2/sros2#282](https://github.com/ros2/sros2/issues/282)). Separate CAs are required.

This is the de-facto functional-safety robotics pattern: per-node X.509 identity, CA-rooted, with least-privilege permissions. It maps almost 1:1 onto Zenoh's native mTLS + ACL.

### 1.2 Zenoh Rust 1.x mTLS (the native fit)
Zenoh's TLS transport supports mutual authentication directly via config (`DEFAULT_CONFIG.json5`, `transport.link.tls`):
- **`enable_mtls`** (bool): requires client certs; server validates against `root_ca_certificate` [Zenoh TLS manual](https://zenoh.io/docs/manual/tls/).
- **`listen_private_key` / `listen_certificate`** (router/server side), **`connect_private_key` / `connect_certificate`** (client side).
- **`root_ca_certificate`**: the fleet CA; if omitted on server side, Mozilla/webpki roots are used (wrong for private fleets).
- **`close_link_on_expiration`** (bool): *"Starting with Zenoh v1.0.3, TLS and QUIC links can be closed when the remote certificate chain expires"* — the local instance monitors the first expiring cert in the remote chain and disconnects the link. Listener-side expiry monitoring **requires `enable_mtls`** (a plain client has no cert to track) [Zenoh TLS manual](https://zenoh.io/docs/manual/tls/), [zenoh-web tls.md](https://github.com/zenoh-rs/zenoh-web/blob/master/content/docs/manual/tls.md).
- **`verify_name_on_connect`** (bool, default true): CN/SAN hostname check.
- **Backend:** Zenoh's TLS/QUIC is built on `rustls` (pulled transitively today via `quinn`/`zenoh-link-commons`, and also via `webrtc`→`dtls`). No new crypto crate needed for mTLS.

### 1.3 Zenoh Access Control (ACL) — least privilege by subject
Zenoh's `access_control` config binds rules to **subjects**. A subject matches on `interfaces`, `cert_common_names`, and `usernames`. `cert_common_names` are matched against the **certificate common name of the remote instance using TLS or QUIC** [Zenoh Access Control manual](https://zenoh.io/docs/manual/access-control/), [DEFAULT_CONFIG.json5](https://github.com/eclipse-zenoh/zenoh/blob/main/DEFAULT_CONFIG.json5). This is how per-robot identity becomes an authorization primitive: ACL policies map `cert_common_names: ["robot-7.fleet"]` → allow `robot/7/**` + `fleet/**`, deny everything else under `default_permission: "deny"`.

### 1.4 iroh (ed25519, CA-less p2p auth)
iroh is a QUIC-based P2P library where **the public key *is* the identity** — no CA:
- Each `Endpoint` holds an Ed25519 `SecretKey`; its `PublicKey` is the `EndpointId` [iroh Endpoints](https://docs.iroh.computer/concepts/endpoints), [iroh keys/EndpointId](https://deepwiki.com/n0-computer/iroh/9.1-keys-and-endpointid).
- Connections are end-to-end encrypted QUIC; dialing is by public key, with optional Pkarr/DNS discovery [iroh blog](https://www.iroh.computer/blog/iroh-dns).
- Auth model is challenge/response at the QUIC layer; there is no built-in server *allowlist* config — allowlisting is an application concern.

### 1.5 rustls
The TLS backend under both Zenoh (QUIC/TLS) and `webrtc` (DTLS) is `rustls` (verified present in the dep tree via `cargo tree -i rustls`: `webrtc→dtls→rustls` and `zenoh→zenoh-link-commons→quinn→rustls`). mTLS rides on rustls with zero new dependencies.

---

## 2. Effort / risk: (a) mTLS vs (b) iroh-style ed25519

### (a) mTLS termination in front of / inside Zenoh
- **What it is:** Configure Zenoh's native TLS transport with `enable_mtls=true`, point `root_ca_certificate` at the fleet CA, ship per-robot cert+key via mounted secret. Optionally enable `close_link_on_expiration`. Bind ACL `cert_common_names` to identities.
- **Effort:** Low. Pure config + a provisioning step. No Rust code change to the TLS path; only `Transport::open_with` must accept a config that enables TLS (already supported). ACL is config-only.
- **New deps:** **None.** rustls already transitive.
- **Risk:** Low. Uses the protocol's own, audited, spec'd auth. Revocation = fleet CA re-issue / cert expiry / ACL update. `close_link_on_expiration` gives automatic link teardown on cert expiry.

### (b) iroh-style ed25519 keypair + server allowlist over Zenoh
- **What it would be:** Each robot holds an Ed25519 keypair; the server keeps an allowlist of robot public keys. Because Zenoh's transport does not natively verify raw ed25519 signatures for admission, you must either:
  - **(b1) Custom nonce-signing handshake** at the application layer: server sends a nonce, robot signs `nonce || server_id`, server verifies against the allowlist. This is a *new protocol* you own, test, and maintain — replay/clock/timing care required.
  - **(b2) ed25519→X.509 bridge:** mint a short-lived X.509 cert from the ed25519 key so Zenoh mTLS can verify it. This needs a cert-signing helper (new code) **and** pulls a crypto/encoding crate (e.g. `rcgen`/`x509-parser`) — **new dependency, requires admin approval per AGENTS.md**.
- **Effort:** Medium–High. Either a from-scratch handshake protocol (b1) or a new crypto dependency + bridge (b2). Significantly more than mTLS.
- **New deps:** **Yes, likely** (b2). ed25519 signing itself is tiny, but a production allowlist + rotation story usually wants `rcgen`/PKI tooling.
- **Risk:** Higher. You own the auth protocol's correctness (replay, nonce freshness, revocation). Zenoh's built-in link-teardown-on-expiry does **not** apply to a custom handshake. Loses the safety net of `close_link_on_expiration` and native ACL `cert_common_names` subjects.
- **Does iroh "drop in cheaply"?** **No.** iroh is a *replacement transport*, not an auth plugin for Zenoh. Dropping iroh in would mean replacing the entire Zenoh transport — out of scope and contrary to the locked Zenoh decision in `src/transport.rs`/`src/rules.rs`. Using only iroh's *auth idea* (ed25519 keypair) over Zenoh still requires (b1) or (b2) above.

### (c) `none`
- Effort: zero. Risk: catastrophic for production (any party on the network can publish `stop/**`, subscribe to `robot/{id}/**`, or impersonate the engine). Acceptable only for loopback/air-gapped dev.

---

## 3. Comparison table

| Axis | **mTLS** (recommended default) | **ed25519 + allowlist** | **none** |
|---|---|---|---|
| Mechanism | Zenoh native TLS `enable_mtls` + fleet CA | ed25519 keypair; custom nonce handshake (b1) or ed25519→X.509 bridge (b2) | no auth |
| Effort | **Low** — config + provisioning | **Medium–High** — new protocol or new crypto dep | **Zero** |
| New deps | **None** (rustls already present) | **Likely** (b2: `rcgen`/PKI, admin approval) | None |
| Risk | **Low** — protocol-native, audited | **Higher** — you own handshake correctness & revocation | **Unacceptable** in prod |
| Safety fit | **Strong** — mirrors SROS2 X.509+CA+perms | Moderate — CA-less; replay/revocation are your burden | None |
| Auto link-teardown on cert expiry | **Yes** (`close_link_on_expiration`, needs mTLS) | No (custom) | n/a |
| Native ACL subject binding | **Yes** (`cert_common_names`) | No (must map pubkey→subject yourself) | n/a |
| Rotation / revocation | Fleet CA re-issue, short-lived certs, ACL update | Allowlist update + key rotation; no native expiry | n/a |
| Best for | Production fleet ↔ cloud engine | Niche CA-less p2p; not a Zenoh drop-in | Loopback / air-gapped dev only |

---

## 4. Recommended default + opt-out axis

**Recommendation: `mTLS` as the default for `flo-engine` client auth.**

- It is protocol-native, requires **no new dependency** (rustls is already in the tree), and reuses the exact SROS2-shaped pattern the functional-safety robotics field already trusts: per-robot X.509 identity → fleet CA → least-privilege permissions.
- It composes with Zenoh's two safety mechanisms for free: ACL `cert_common_names` subjects for authorization, and `close_link_on_expiration` for automatic teardown when a robot's cert expires.

Expose an explicit axis in `flo` config:

```toml
[engine.auth]
mode = "mtls"          # none | mtls | ed25519
allow_insecure = false # hard gate; only consulted when mode = "none"
```

Semantics:
- **`mtls`** (default): `enable_mtls=true`, `root_ca_certificate` = fleet CA, per-robot cert+key from mounted secret, `close_link_on_expiration=true`, ACL `default_permission="deny"` with `cert_common_names` policies.
- **`ed25519`**: opt-in escape hatch for CA-less deployments; implement (b1) nonce-signing handshake + server allowlist. Document that this owns its own replay/revocation story and is **not** a Zenoh-native path.
- **`none`**: loopback / air-gapped dev **only**. `Transport::open` must **refuse to start** unless `allow_insecure = true` is explicitly set; production deployments that set `mode = "none"` without `allow_insecure` must hard-fail at startup with a clear error. The cloud engine must likewise reject `none` connections unless an explicit `allow_insecure` flag is present server-side.

---

## 5. Spec mandates (non-negotiable for `mtls`)

1. **Per-robot identity at provisioning.** Each robot container is issued a unique X.509 cert+key (CN = `robot-<id>.fleet` or similar) at provisioning time, signed by the fleet Identity CA. Mirrors SROS2 per-enclave identity [ROS 2 DDS-Security](https://design.ros2.org/articles/ros2_dds_security.html).
2. **Fleet CA / server allowlist.** A dedicated fleet CA (separate from any Permissions CA, per the SROS2 single-CA lesson [ros2/sros2#282](https://github.com/ros2/sros2/issues/282)) signs robot certs. The cloud engine's `root_ca_certificate` pins that CA. Revocation = CA-level (OCSP/CRL or short-lived certs + `close_link_on_expiration`).
3. **Mounted secret storage — never images.** Cert+key are mounted as Kubernetes Secrets / files at runtime; **never** baked into the container image or committed to the repo (AGENTS.md: `.env` files not used for secrets; same principle for certs).
4. **Zenoh ACL least-privilege.** `access_control.enabled=true`, `default_permission="deny"`. Policies bind `cert_common_names` to key-expression scopes: a robot may `put`/`declare_subscriber` only on `robot/<its-id>/**` and required `fleet/**` topics; `stop/**` (class 1) and `lidar/**` (class 2) scoped to authorized actuators/sensors [Zenoh ACL](https://zenoh.io/docs/manual/access-control/).
5. **Rotation / revocation.** Short-lived robot certs (e.g. days) with automated re-issue; `close_link_on_expiration=true` ensures stale links die. Revoked robots are removed from the CA/ACL allowlist; engine rejects their certs at handshake.

---

## 6. References

- Zenoh TLS authentication (mTLS, `close_link_on_expiration`, v1.0.3): https://zenoh.io/docs/manual/tls/
- Zenoh access control (`cert_common_names` subjects, `default_permission`): https://zenoh.io/docs/manual/access-control/
- Zenoh default config (`transport.link.tls`, `access_control`, `auth`): https://github.com/eclipse-zenoh/zenoh/blob/main/DEFAULT_CONFIG.json5
- Zenoh crate / features (`auth_pubkey`, `auth_usrpwd`, `transport_tls`, `transport_quic` on by default): https://docs.rs/zenoh/1.9.0/zenoh/
- Zenoh protocol — Authentication: https://spec.zenoh.io/spec/1.0.0/security/authentication.html
- ROS 2 DDS-Security / SROS2 design: https://design.ros2.org/articles/ros2_dds_security.html
- SROS2 single-CA chain-of-trust pitfall: https://github.com/ros2/sros2/issues/282
- OpenDDS DDS-Security (X.509 + permissions/governance): https://opendds.readthedocs.io/en/latest-release/devguide/dds_security.html
- iroh Endpoints / Ed25519 identity: https://docs.iroh.computer/concepts/endpoints
- iroh keys & EndpointId (Ed25519, no CA): https://deepwiki.com/n0-computer/iroh/9.1-keys-and-endpointid
- iroh dial-by-NodeId / Pkarr: https://www.iroh.computer/blog/iroh-dns
- rustls (TLS backend under Zenoh QUIC + webrtc DTLS): https://docs.rs/rustls/
