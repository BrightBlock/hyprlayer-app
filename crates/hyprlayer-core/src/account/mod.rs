//! Hyprlayer Cloud sign-in: Cognito federated to GitHub, held by the server (ADR-0011).

pub(crate) mod cognito;
pub mod endpoints;
pub mod loopback;
pub(crate) mod pkce;
pub(crate) mod store;
