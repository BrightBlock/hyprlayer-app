//! The Hyprlayer runtime library.
//!
//! # Supported API
//!
//! The supported surface is the [`HyprlayerRuntime`] facade and nothing else:
//! construct it with [`RuntimeOptions`] over an explicit [`RuntimeEnvironment`]
//! and [`RuntimePaths`], drive it through the eleven service handles re-exported
//! below, observe [`RuntimeEvent`] through [`HyprlayerRuntime::subscribe`], and
//! stop it with [`HyprlayerRuntime::shutdown`]. Errors are [`CoreError`] and the
//! crate's [`Result`] alias; durable values are [`models`].
//!
//! Each service's operational request/result types live beside it, in its
//! `runtime::<service>` module, and are supported along with the handle:
//! [`runtime::settings_service`], [`runtime::providers`], [`runtime::sessions`],
//! [`runtime::projects`], [`runtime::library`], [`runtime::agents`],
//! [`runtime::prompts`], [`runtime::workstreams`], [`runtime::attachments`],
//! [`runtime::terminals`], [`runtime::scripts`].
//!
//! `docs/architecture/rust-runtime-library.md` is the consumer-facing guide.
//!
//! # Not supported API
//!
//! Every other `pub mod` below is implementation detail with no stability
//! promise — provider protocols, raw stores, concrete registries, watchers,
//! Git/process implementations, and driver internals. They are public only
//! because `hyprlayer-app` still reaches through them for the desktop carve-outs
//! ADR-0008 names; that is a migration aid, not a contract. Renamed fields and
//! inherent methods may change under any of them without notice.

pub mod agent;
pub mod agent_bundle;
pub mod agents;
pub mod branch_prefix;
pub mod capability;
pub mod claude;
pub mod codex;
pub mod config;
pub mod docs;
pub mod env;
pub mod error;
pub mod gitops;
pub mod hyprcli;
pub mod index;
pub mod omarchy;
pub mod opencode;
pub mod pipeline;
pub mod pr;
pub mod process;
pub mod redact;
pub mod repo_settings;
pub mod repos;
pub mod runtime;
pub mod scripts;
pub mod settings;
pub mod skills;
pub mod store;
pub mod system;
pub mod terminal;
pub mod thoughts_repo;
pub mod worktree;

pub use error::{CoreError, Result};
pub use hyprlayer_models as models;

pub use env::{RuntimeEnvironment, RuntimePaths};
pub use models::{
    Harness, LogCursor, PermissionMode, ProjectRecord, ProjectSource, RuntimeEvent, ScriptKind,
    Session, SessionEvent, SessionLogEntry, SessionState, Stage, Workstream, WorkstreamDoc,
    WorkstreamStatus, WorktreeMode,
};
pub use runtime::shutdown::ShutdownReport;
pub use runtime::{HyprlayerRuntime, RuntimeOptions};

pub use runtime::agents::AgentService;
pub use runtime::attachments::AttachmentService;
pub use runtime::library::LibraryService;
pub use runtime::projects::ProjectService;
pub use runtime::prompts::PromptService;
pub use runtime::providers::ProviderService;
pub use runtime::scripts::ScriptService;
pub use runtime::sessions::SessionService;
pub use runtime::settings_service::SettingsService;
pub use runtime::terminals::TerminalService;
pub use runtime::workstreams::WorkstreamService;
