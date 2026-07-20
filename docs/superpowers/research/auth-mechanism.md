# flo-engine Client Authentication Mechanism — Research Findings

- **Ticket:** #64 — Client auth mechanism for flo-engine (mTLS vs ed25519 vs none, opt-out)
- **Date:** 2026-07-20
- **Scope:** Cloud `flo-engine` (Zenoh router + rule service) admitting robot clients (containers on physical robots).
- **Constraint:** Auth axis MUST be `auth: none | mtls | ed25519` (opt-out). Existing `flo` transport is Zenoh-based (`src/transport.rs`, `src/rules.rs`).

## Recommended Default

**Default = `mtls`** (mutual TLS, terminated natively by the Zenoh link layer), with `none` and `ed25519` as explicit opt-in/opt-out alternatives.

Rationale in one line: Zenoh 1.x already supports mTLS as a first-class, config-only link feature with zero custom protocol code, giving strongest safety fit (identity + encryption + expiry-closed links) at the lowest engineering and audit risk; `ed25519` is viable but requires designing, testing, and securing a custom auth handshake over Zenoh (or a `rustls` wrapper), which is a larger, riskier surface for a functional-safety fleet.

---

## State of the Art (Survey)

### 1. ROS 2 DDS-Security (functional-safety field robotics baseline)
- DDS-Security SPI defines five plugins: **Authentication**, **Access Control**, **Cryptographic**, **Logging**, **Data Tagging**; ROS 2 / SROS2 uses Authentication + Access Control + Cryptographic [ROS 2 DDS-Security design](https://design.ros2.org/articles/ros2_dds_security.html).
- Authentication is **X.509 PKI** (Identity CA + Permissions CA), encryption is **AES-GCM**, access control is per-topic governance/permissions XML [RMF Security chapter](https://osrf.github.io/ros2multirobotbook/security.html).
- SROS2 ships tooling (keystore, governance, permissions generation) and a master on/off switch — directly analogous to our `auth: none` opt-out semantics [SROS2](https://design.ros2.org/articles/ros2_dds_security.html).
- **Lesson for flo:** the proven functional-safety model is per-client X.509 identity + least-privilege topic ACL. mTLS maps onto this directly. ed25519 (no PKI) trades the CA rotation/revocation machinery for simpler key management but loses standard cert-expiry/disjoint CA semantics.

### 2. Zenoh Rust auth / link layers
- **TLS link** (native): Zenoh supports one-way TLS and mutual TLS (`enable_mtls: true`) on `tls/...` endpoints. Client/router configure `root_ca_certificate`, `listen_private_key`, `listen_certificate`, `connect_private_key`, `connect_certificate` [Zenoh TLS manual](https://zenoh.io/docs/manual/tls).
- **Certificate expiry enforcement:** since Zenoh v1.0.3, TLS/QUIC links close automatically when the remote cert chain expires (only enforced with mTLS) [Zenoh TLS manual](https://zenoh.io/docs/manual/tls).
- **User-Password auth:** built-in transport-layer `usrpwd` with a server-side `dictionary_file` allowlist [Zenoh user-password manual](https://zenoh.io/docs/manual/user-password/).
- **Access Control:** Zenoh 1.x ACL subjects match `cert_common_names` (TLS/QUIC), `usernames`, and `interfaces`; rules apply per key-expression, message type, and ingress/egress flow [Zenoh access-control manual](https://zenoh.io/docs/manual/access-control/). This means mTLS identity (`cert_common_names`) can drive least-privilege topic authorization natively.
- **Pluggability:** auth is part of the config tree (`transport/auth`, `transport/link/tls`); the `flo` crate can inject it via `zenoh::Config::insert_json5` (as `Transport::open_with` already does in `src/transport.rs:41`). No custom protocol needed.
- **Important:** Zenoh's built-in auth is cert/PSK based. There is **no built-in ed25519-keypair allowlist auth plugin** in upstream Zenoh — an ed25519 scheme must be either (a) a custom application-layer handshake, or (b) a `rustls`/TLS wrapper where the ed25519 key is minted into a self-signed X.509 cert (the iroh pattern, see below).

### 3. rustls (mTLS)
- `rustls` is the de-facto safe-Rust TLS stack (no `unsafe` in our code; `flo` is `#![forbid(unsafe_code)]`). Zenoh's TLS link is built on rustls. Choosing mTLS means we ride Zenoh's rustls integration — no direct `rustls` dependency added to `flo` unless we terminate TLS ourselves in front of Zenoh.
- Trade-off: if we terminate mTLS in a sidecar/proxy (e.g. `axum`-based gateway, since `flo` already depends on `axum`) we own cert validation; if we let Zenoh terminate it, we only configure it. The latter is strongly preferred for effort/risk.

### 4. iroh (ed25519 keypair + node-id, passwordless p2p auth)
- iroh nodes are identified by an **ed25519 keypair**; the public key *is* the node id. In iroh's QUIC transport, the ed25519 key is used as the TLS certificate (self-signed, no PKI), and the handshake authenticates it — "no PKI: an incoming connection presents its ED25519 public key, the TLS handshake makes sure it's authentic, and it's up to the application whether it trusts that key" [mushi / iroh pattern](https://lib.rs/crates/mushi).
- This is exactly the `ed25519` axis: per-robot ed25519 keypair, server holds an **allowlist of public keys**; trust is application-defined.
- **Drop-in cost:** iroh's primitives do **not** drop into Zenoh for free. iroh is its own QUIC/relay networking stack. Mapping iroh-style ed25519 auth onto a Zenoh transport means either (a) replacing Zenoh's transport with iroh (out of scope — breaks the existing `src/transport.rs` key-expression model), or (b) designing a custom auth handshake: client signs a server-provided nonce with its ed25519 key over a Zenoh put/get, server verifies against the allowlist and gates the session. That handshake is **new protocol surface to design, implement, test, and security-review** — a meaningful risk for a safety-critical fleet.
- Crypto crates are available and mature (`ed25519-dalek` 3.x, `ed25519` signature traits, `ring`) but `flo` does **not** currently depend on any of them (see below).

---

## Effort / Risk Analysis

| Axis | mTLS (rustls, Zenoh-terminated) | ed25519 (iroh-style keypair + allowlist) | none |
|---|---|---|---|
| **Implementation effort** | Low — config-only via Zenoh TLS link + `enable_mtls`; reuse `Transport::open_with`. | High — design + implement + test a custom nonce-signing auth handshake over Zenoh, OR mint ed25519→X.509 and reuse Zenoh mTLS. | Trivial — no auth. |
| **Crypto we own** | None (Zenoh/rustls). | We own signing/verification + handshake logic (attack surface). | None. |
| **Transport encryption** | Yes (TLS, AES-GCM class). | Only if we also wrap TLS; raw ed25519 handshake authenticates but does not encrypt by itself. | No. |
| **Identity / spoofing resistance** | Strong (per-client cert, CA-signed). | Strong (per-robot keypair, allowlist). | None. |
| **Revocation / rotation** | Cert expiry auto-closes link (v1.0.3+); CA reissue. | Must rotate allowlist + redeploy; no built-in expiry. | N/A. |
| **Least-privilege ACL** | Native via Zenoh ACL `cert_common_names` subjects. | Must map pubkey→subject ourselves; ACL is manual. | N/A. |
| **Supply-chain / audit risk** | Low (relies on audited Zenoh/rustls). | Medium (custom protocol + new crypto crates: `ed25519-dalek`/`ring`). | High (open fleet). |
| **Functional-safety fit** | Best (X.509 PKI mirrors ROS 2 DDS-Security norm). | Good, but non-standard for field robotics. | Unsafe for production. |
| **New dependencies in `flo`** | None (Zenoh pulls rustls transitively). | Adds `ed25519-dalek` (or `ring`) + handshake module — needs admin approval per AGENTS.md. | None. |

### Current `flo` dependencies (from `Cargo.toml`)
`axum 0.8`, `anyhow`, `serde`, `tokio`, `toml`, `tracing`, `webrtc 0.17`, **`zenoh 1.9`** (`unstable` feature), `bytes`, `clap`; `gstreamer*` optional behind `media`.
- **No `rustls`, `ring`, or `ed25519` crate is a direct dependency.** They are *transitive* (Zenoh → rustls). So mTLS adds **zero** new first-party deps; ed25519 adds at least one new crypto crate (requires admin approval per AGENTS.md dependency policy).
- This tilts effort/risk decisively toward mTLS: it is the only axis that needs no new dependency and no new protocol.

---

## Mapping onto Zenoh

- **mTLS:** Native. Set `listen`/`connect` endpoints to `tls/...`, set `transport.link.tls.enable_mtls = true`, provide CA + per-robot cert/key. Server closes expired links automatically. Gate topics via Zenoh ACL using `cert_common_names`. No custom code.
- **ed25519:** Not native. Two paths:
  1. *Custom handshake (pure Zenoh):* on connect, client `put`s a signed nonce to a `robot/{id}/auth/challenge` key-expr; a server-side subscriber verifies the ed25519 signature against the allowlist before admitting rule traffic. Must also encrypt (else auth-only, no confidentiality). This is **new, security-critical protocol code** to design/test/audit.
  2. *ed25519→X.509 bridge:* generate a self-signed cert from the ed25519 key at provisioning (iroh's own trick) and feed it to the *same* Zenoh mTLS link. This collapses the ed25519 axis into the mTLS machinery — lower risk, but then "ed25519" is just a key-format choice over mTLS, not a distinct transport.
- **none:** `auth: none` → plain `tcp/` endpoints, no TLS, no handshake. Permitted only for air-gapped/loopback demos (matches today's `Transport::loopback_config` dev path).

**Feasibility verdict:** mTLS = drop-in. ed25519 = feasible but requires a custom handshake (path 1) or degenerates into mTLS-with-ed25519-certs (path 2). A "custom auth handshake" is the dominant risk and is the reason mTLS is the recommended default.

---

## Provisioning Model (what the spec MUST mandate)

Applies to `mtls` (and analogously to `ed25519` if selected):

1. **Per-robot identity:** each robot container gets a unique credential at provisioning time — an X.509 client cert/key (mTLS) or ed25519 keypair (ed25519). Identity is the robot's stable id used in key-expressions (`robot/{id}/...`).
2. **CA / root of trust:** a fleet CA signs client certs (mTLS). Server holds the root CA; clients hold the server's cert for server-auth. For ed25519, the server holds the **allowlist** of robot public keys (out-of-band, versioned).
3. **Key/cert storage:** secrets live in the robot container's mounted secret volume (Kubernetes Secret / mounted file), **never** baked into images. Private keys marked read-only, non-exportable where possible. `#![forbid(unsafe_code)]` stays; no secret logging.
4. **Server allowlist:** for mTLS, the CA constrains who can connect; Zenoh ACL `cert_common_names` maps cert → allowed key-expressions (least privilege: a robot may only write `robot/{its_id}/**` and read `fleet/**` as permitted). For ed25519, an explicit versioned allowlist file/dict.
5. **Rotation & revocation:** certs carry expiry; Zenoh auto-closes expired links (v1.0.3+). CA re-issue + cert rollover procedure documented. For ed25519, allowlist update + redeploy path required (no native expiry).
6. **Opt-out semantics:** `auth: none` is **only** valid for loopback/air-gapped dev (same trust level as `Transport::loopback_config`). Production configs MUST NOT ship `none`; the engine should refuse to start in `none` mode unless an explicit `allow_insecure: true` override + non-routable bind is set. `mtls` is the default when `auth` is unset.

---

## Opt-out Semantics Summary

| `auth` value | Behavior | Production-safe? |
|---|---|---|
| `mtls` (default) | Zenoh TLS link, `enable_mtls`, CA + per-robot cert, ACL by `cert_common_names`. | Yes (default). |
| `ed25519` | ed25519 keypair + server allowlist (custom handshake or ed25519→X.509 bridge). | Yes, if handshake implemented & audited. |
| `none` | No auth/encryption; loopback/air-gapped only. | No — blocked unless `allow_insecure` + local bind. |

---

## References

- Zenoh TLS authentication (mTLS, cert expiry): https://zenoh.io/docs/manual/tls
- Zenoh User-Password auth: https://zenoh.io/docs/manual/user-password/
- Zenoh Access Control (subjects, `cert_common_names`): https://zenoh.io/docs/manual/access-control/
- Zenoh Protocol Spec — Security/Authentication: https://spec.zenoh.io/spec/1.0.0/security/index.html , https://spec.zenoh.io/spec/1.0.0/security/authentication.html
- Zenoh Rust `Config`: https://docs.rs/zenoh/latest/zenoh/struct.Config.html
- ROS 2 DDS-Security design (SROS2, X.509 PKI, AES-GCM): https://design.ros2.org/articles/ros2_dds_security.html
- RMF / ROS 2 Security (DDS-Security SPI, SROS2 tooling): https://osrf.github.io/ros2multirobotbook/security.html
- iroh ed25519 node-id / keypair model: https://docs.rs/iroh/latest/iroh/key/index.html
- iroh ed25519-as-cert pattern (mushi writeup): https://lib.rs/crates/mushi
- ed25519 Rust crypto (`ed25519-dalek`, `ed25519` traits): https://lib.rs/crates/ed25519 , https://lib.rs/cryptography
- Zenoh GitHub (router, TLS plugin `zenoh-link-tls`): https://github.com/eclipse-zenoh/zenoh , https://docs.rs/zenoh-link-tls

---

## Decision

Adopt **`auth: none | mtls | ed25519`** with **`mtls` as the default**. mTLS is the only axis that is config-only (Zenoh-native), adds no new dependency to `flo`, encrypts by default, auto-expires links, and natively drives least-privilege ACLs — matching the ROS 2 DDS-Security functional-safety norm at the lowest engineering and audit risk. `ed25519` remains a first-class opt-in for fleets that prefer key-based allowlists, but its safe realization requires a custom handshake (or ed25519→X.509 bridge) that the spec must scope and security-review before enabling. `none` is a dev/air-gapped opt-out, hard-blocked in production without an explicit override.
