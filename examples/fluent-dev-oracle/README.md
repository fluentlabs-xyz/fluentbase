<div align="center">

<img src="https://iili.io/BgpTM4n.jpg" alt="Fluent Dev Oracle" width="120" height="120" style="border-radius: 16px;"/>

# 🔮 fluent-dev-oracle

**Decentralized Developer Identity Registry** — Built on the Fluent L2 Network using rWasm & Blended Execution.

[![Status](https://img.shields.io/badge/status-alpha-orange?style=for-the-badge)](https://github.com/FreeDropOracle/fluent-dev-oracle)
[![Version](https://img.shields.io/badge/version-v0.1.0--alpha-blue?style=for-the-badge)](https://github.com/FreeDropOracle/fluent-dev-oracle)
[![PR](https://img.shields.io/badge/PR-%23398-purple?style=for-the-badge)](https://github.com/FreeDropOracle/fluent-dev-oracle/pull/398)
[![License](https://img.shields.io/badge/license-MIT-green?style=for-the-badge)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-no__std-black?style=for-the-badge&logo=rust)](https://www.rust-lang.org/)

<br/>

> *"Architecting Truth on the Fluent Layer."*
> 
> — [@freedroporacle](https://github.com/freedroporacle)

</div>

---

## 📖 Table of Contents

- [Executive Summary](#-executive-summary)
- [Tech Stack](#️-tech-stack)
- [Architecture](#-architecture)
- [Data Flow](#-secure-data-flow)
- [Project Structure](#-project-structure)
- [Security Features](#️-security-features)
- [Roadmap](#-known-risks--roadmap)
- [Links](#-official-links)

---

## 🎯 Executive Summary

`fluent-dev-oracle` is a high-performance reference implementation of a **decentralized identity registry** for the Fluent L2 ecosystem. It creates an **immutable, verifiable, cryptographic mapping** between GitHub Repository Hashes and Developer Wallet Addresses — bringing on-chain accountability to open-source development.

```
GitHub Repository URL  ──►  keccak256 Hash  ──►  Wallet Address  ──►  Fluent L2 Storage
```

Key design goals:

- ⚡ **ZK-Proof Friendly** — Minimized execution trace for low L2 proving costs
- 🔒 **Collision-Proof** — Namespaced storage prevents EVM/Wasm slot overlap
- 🌐 **Blended Execution** — Runs natively in Fluent's rWasm environment
- 🔗 **Immutable** — Write-once, read-forever identity anchoring

---

## 🛠️ Tech Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Language** | `Rust` (Edition 2021) | Zero-cost abstractions, memory safety |
| **Runtime** | `wasm32-unknown-unknown` → rWasm IR | Deterministic L2 execution |
| **Core SDK** | `fluentbase-sdk` v1.1.7 | Fluent-native storage & crypto APIs |
| **Cryptography** | `alloy-primitives` · `keccak256` | Hash derivation & address encoding |
| **Environment** | `no_std` | Zero OS-dependency for ZK determinism |
| **Frontend** | `Next.js 15` · `wagmi` · `viem` | Web3 UI with MetaMask integration |

---

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        FLUENT L2 NETWORK                        │
│                                                                 │
│   ┌─────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│   │   Frontend  │    │  rWasm Smart │    │  Fluent Storage  │  │
│   │  (Next.js)  │───►│   Contract   │───►│   (Key-Value)    │  │
│   │             │    │  (Rust/wasm) │    │                  │  │
│   └─────────────┘    └──────────────┘    └──────────────────┘  │
│          │                  │                      │            │
│     MetaMask           ECDSA Verify           ZK-Provable       │
│     Signature          + keccak256            State Root        │
└─────────────────────────────────────────────────────────────────┘
```

### Identity Lifecycle

```
                    ┌──────────────────────────────┐
                    │      Developer Journey        │
                    └──────────────────────────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
     ┌─────────────────┐  ┌─────────────────┐  ┌──────────────────┐
     │  1. INPUT       │  │  2. HASH        │  │  3. REGISTER     │
     │                 │  │                 │  │                  │
     │  github.com/    │  │  keccak256(     │  │  sign_message(   │
     │  user/repo      │─►│    prefix +     │─►│    challenge     │
     │                 │  │    repoHash     │  │  ) → signature   │
     └─────────────────┘  │  ) = slot_key   │  └──────────────────┘
                          └─────────────────┘           │
                                                        ▼
                                               ┌─────────────────┐
                                               │  4. ON-CHAIN    │
                                               │                 │
                                               │  storage[key] = │
                                               │  wallet_address │
                                               │                 │
                                               │  ✅ IMMUTABLE   │
                                               └─────────────────┘
```

---

## 🔄 Secure Data Flow

### VM Execution Graph

```
┌─────────────┐
│VM Invocation│
└──────┬──────┘
       │
       ▼
┌─────────────────┐     ┌──────────────────────────────────────┐
│ main_entry()    │────►│         sdk.bytes_input()            │
└─────────────────┘     └──────────────────┬───────────────────┘
                                           │
                               ┌───────────▼──────────┐
                               │  Input >= 32 bytes?  │
                               └───────────┬──────────┘
                                     YES   │   NO
                          ┌──────────────┘  └──────────────────┐
                          ▼                                     ▼
              ┌───────────────────────┐              ┌──────────────────┐
              │  Derive Storage Key   │              │  REVERT / NOOP   │
              │  keccak256(           │              └──────────────────┘
              │    PREFIX + InputHash │
              │  )                    │
              └───────────┬───────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  contract_caller()    │
              │  → Wallet Address     │
              └───────────┬───────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  write_storage(       │
              │    key   → slot_hash  │
              │    value → address    │
              │  )                    │
              └───────────┬───────────┘
                          │
                          ▼
              ┌───────────────────────┐
              │  emit_log(            │
              │    "registered: ..."  │
              │  )                    │
              └───────────────────────┘
```

### Data Transformation Matrix

| Stage | Input | Transformation | Output | Security Benefit |
|-------|-------|----------------|--------|-----------------|
| **Parse** | Raw bytes | `bytes_input()` | `[u8; 32]` | Type-safe deserialization |
| **Key Derivation** | `RepoHash` | `keccak256(PREFIX ‖ hash)` | `U256 slot` | Namespace isolation |
| **Identity** | EVM context | `contract_caller()` | `Address` | Verifiable origin |
| **Commit** | `(slot, address)` | `write_storage()` | State delta | Immutable on-chain record |
| **Broadcast** | Identity record | `emit_log()` | String event | Off-chain oracle indexing |

---

## 📁 Project Structure

```bash
fluent-dev-oracle/
│
├── 📄 Cargo.toml              # Manifest with strict dependency isolation
├── 📄 vercel.json             # Frontend deployment config
├── 📄 README.md               # You are here
│
├── 📂 src/
│   └── lib.rs                 # Core registry logic & rWasm entrypoint
│
├── 📂 frontend/               # Next.js 15 Web3 Interface
│   ├── 📂 src/
│   │   ├── app/
│   │   │   ├── registry/      # Identity registration UI
│   │   │   ├── explorer/      # Verification explorer
│   │   │   └── academy/       # FLIP-007 documentation
│   │   ├── components/
│   │   │   └── navigation.tsx # Wallet connect + routing
│   │   └── lib/
│   │       ├── wagmi-config.ts # Web3 provider config
│   │       ├── hash-utils.ts   # keccak256 utilities
│   │       └── contract.ts     # Contract interface
│   └── 📄 package.json
│
├── 📂 docs/                   # Architecture & mind maps
└── 📂 contract/               # Additional contract artifacts
```

---

## 🛡️ Security Features

### ✅ Implemented Protections

#### 1. Namespace Isolation
```
Storage Key = keccak256("fluent.oracle.dev_identity.v1" ‖ repoHash)
```
Using a unique domain prefix prevents storage slot collisions between:
- Oracle storage ↔ ERC-20 storage
- EVM contracts ↔ rWasm contracts
- Different oracle versions

#### 2. ECDSA Challenge-Response *(Frontend Layer)*
```
challenge = keccak256(repoHash ‖ walletAddress ‖ timestamp)
signature = MetaMask.sign(challenge)
```
Proves wallet ownership at registration time.

#### 3. Read-Before-Write Semantics
The contract enforces strict RBW — attempting to overwrite an existing registration is rejected at the VM level, not just the frontend.

#### 4. Deterministic Key Derivation
All storage keys are derived via `keccak256`, making every slot:
- Predictable for ZK-provers
- Reproducible for off-chain verification
- Collision-resistant by construction

#### 5. `no_std` Execution Environment
Zero standard library dependency ensures:
- Fully deterministic execution traces
- No hidden syscalls or OS-level entropy
- Minimal proving overhead for L2 validators

---

## ⚠️ Known Risks & Roadmap

| # | Risk | Severity | Description | Mitigation |
|---|------|----------|-------------|------------|
| 1 | **Access Control** | 🟡 Medium | Repo ownership not cryptographically verified | Integrate GitHub OAuth + signed commits |
| 2 | **Event Structure** | 🟢 Low | Logs are raw strings | Standardize via `codec` crate for indexers |
| 3 | **Replay Protection** | 🟡 Medium | Timestamp window is ±5min | Tighten to ±60s + nonce registry |
| 4 | **Upgradeability** | 🔴 High | No proxy pattern — immutable at v0.1 | Design upgrade path for v1.0 |

### 🗺️ Version Roadmap

```
v0.1.0-alpha  ──────────────────────────────────────────────────►
     │              │              │              │              │
  [NOW]          [v0.2]         [v0.3]         [v0.5]        [v1.0]
     │              │              │              │              │
  ✅ Core        🔄 GitHub     🔄 Codec       🔄 Multi-     🔄 Proxy
  Registry       OAuth         Events         chain         Pattern
  ✅ Frontend    🔄 Nonce      🔄 Indexer     🔄 DAO        🔄 Mainnet
  ✅ MetaMask    Registry      Support        Governance    Deploy
```

---

## 🧠 Architecture Mind Map

```
                         ┌──────────────────┐
                         │  fluent-dev-oracle│
                         └────────┬─────────┘
                                  │
          ┌───────────────────────┼───────────────────────┐
          │                       │                       │
   ┌──────▼──────┐       ┌────────▼───────┐      ┌───────▼───────┐
   │  IDENTITY   │       │   SECURITY     │      │  PERFORMANCE  │
   └──────┬──────┘       └────────┬───────┘      └───────┬───────┘
          │                       │                      │
   ┌──────▼──────┐         ┌──────▼──────┐       ┌──────▼──────┐
   │ Repo Hash   │         │  keccak256  │       │  no_std     │
   │ Wallet Addr │         │  Namespace  │       │  rWasm IR   │
   │ Timestamp   │         │  ECDSA Sig  │       │  ZK-Friendly│
   └─────────────┘         └─────────────┘       └─────────────┘
```

---

## 📝 Key Technical Observations

> **rWasm Optimized**
> Uses `NativeCasAllocator` — specifically tuned for rWasm execution, eliminating standard library overhead for maximum determinism.

> **ZK-Proof Friendly**  
> Flattened Key-Value storage structure minimizes execution trace complexity, directly reducing proving costs for Fluent L2 validators.

> **Blended Execution Ready**  
> Designed to coexist with EVM contracts in Fluent's Unified Account Space without storage interference.

---

## 🔗 Official Links

| Resource | Link |
|----------|------|
| 🌐 Live App | [fluent-dev-oracle.vercel.app](https://fluent-dev-oracle.vercel.app) |
| 📚 Fluent Docs | [docs.fluentlabs.xyz](https://docs.fluentlabs.xyz) |
| 🔧 Fluentbase SDK | [github.com/fluentlabs-xyz/fluentbase](https://github.com/fluentlabs-xyz/fluentbase) |
| 📋 FLIP-007 Standard | [Academy Page](https://fluent-dev-oracle.vercel.app/academy) |

---

<div align="center">

```
██████╗ ███████╗██╗   ██╗    ██╗██████╗ ███████╗███╗   ██╗████████╗██╗████████╗██╗   ██╗
██╔══██╗██╔════╝██║   ██║    ██║██╔══██╗██╔════╝████╗  ██║╚══██╔══╝██║╚══██╔══╝╚██╗ ██╔╝
██║  ██║█████╗  ██║   ██║    ██║██║  ██║█████╗  ██╔██╗ ██║   ██║   ██║   ██║    ╚████╔╝ 
██║  ██║██╔══╝  ╚██╗ ██╔╝    ██║██║  ██║██╔══╝  ██║╚██╗██║   ██║   ██║   ██║     ╚██╔╝  
██████╔╝███████╗ ╚████╔╝     ██║██████╔╝███████╗██║ ╚████║   ██║   ██║   ██║      ██║   
╚═════╝ ╚══════╝  ╚═══╝      ╚═╝╚═════╝ ╚══════╝╚═╝  ╚═══╝   ╚═╝   ╚═╝   ╚═╝      ╚═╝  
```

**Developed with 🔮 by [@freedroporacle](https://github.com/freedroporacle)**

*Architecting Truth on the Fluent Layer.*

![Fluent Network](https://img.shields.io/badge/Powered%20by-Fluent%20Network-6366f1?style=for-the-badge)
![rWasm](https://img.shields.io/badge/Runtime-rWasm-orange?style=for-the-badge)
![ZK Ready](https://img.shields.io/badge/ZK-Proof%20Ready-green?style=for-the-badge)

</div>
