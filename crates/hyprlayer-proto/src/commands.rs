//! The command-table generator. Each `Variant: method(args) -> Ret;` line yields
//! a `Request` variant, a `Response` variant, a `Dispatch` method, and a client
//! method; the table itself is instantiated where its argument and result types
//! are in scope, because this crate depends on nothing but `serde`.

/// Generates `Request`, `Response`, `Dispatch`, and `impl $client` from one
/// line per command. `$client` supplies
/// `async fn call(&self, Request) -> Result<Response, WireError>`.
///
/// Both enums are externally tagged (`{"echo":{"text":"hi"}}` in, `{"echo":"HI"}`
/// out), so a large body deserializes straight into its type rather than
/// through serde's buffered `Content`. Streaming commands are not expressible
/// here yet.
#[macro_export]
macro_rules! commands {
    (
        client = $client:ident;
        $( $variant:ident : $method:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty ; )*
    ) => {
        // One variant per command means the largest body sizes the enum; both
        // are transient values that are never boxed into anything long-lived.
        #[allow(clippy::large_enum_variant)]
        #[derive(Debug, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Request {
            $( $variant { $( $arg: $ty ),* }, )*
        }

        impl Request {
            pub fn command(&self) -> &'static str {
                match self {
                    $( Request::$variant { .. } => stringify!($method), )*
                }
            }
        }

        #[allow(clippy::large_enum_variant)]
        #[derive(Debug, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Response {
            $( $variant($ret), )*
        }

        impl Response {
            pub fn command(&self) -> &'static str {
                match self {
                    $( Response::$variant(_) => stringify!($method), )*
                }
            }
        }

        pub trait Dispatch {
            $(
                fn $method(
                    &self,
                    $( $arg: $ty ),*
                ) -> impl ::core::future::Future<Output = ::core::result::Result<$ret, $crate::WireError>> + Send;
            )*

            fn dispatch(
                &self,
                request: Request,
            ) -> impl ::core::future::Future<Output = ::core::result::Result<Response, $crate::WireError>> + Send
            where
                Self: Sync,
            {
                async move {
                    match request {
                        $( Request::$variant { $( $arg ),* } => {
                            self.$method($( $arg ),*).await.map(Response::$variant)
                        } )*
                    }
                }
            }
        }

        impl $client {
            $(
                pub async fn $method(
                    &self,
                    $( $arg: $ty ),*
                ) -> ::core::result::Result<$ret, $crate::WireError> {
                    match self.call(Request::$variant { $( $arg ),* }).await? {
                        Response::$variant(value) => Ok(value),
                        other => Err($crate::WireError::protocol(format!(
                            "{} was answered with a {} response",
                            stringify!($method),
                            other.command()
                        ))),
                    }
                }
            )*
        }
    };
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use crate::WireError;

    crate::commands! {
        client = Client;
        Ping: ping(n: u32) -> u32;
        Echo: echo(text: String, upper: bool) -> String;
        Reset: reset() -> ();
    }

    struct Server;

    impl Dispatch for Server {
        async fn ping(&self, n: u32) -> Result<u32, WireError> {
            Ok(n + 1)
        }

        async fn echo(&self, text: String, upper: bool) -> Result<String, WireError> {
            Ok(if upper { text.to_uppercase() } else { text })
        }

        async fn reset(&self) -> Result<(), WireError> {
            Err(WireError {
                code: "reset_refused".into(),
                message: "no".into(),
                remedy: None,
            })
        }
    }

    /// The generated client methods only need `call`; here it is an in-process
    /// hop through the JSON both sides would put on the wire.
    struct Client {
        answer_everything_with: Option<Response>,
    }

    impl Client {
        async fn call(&self, request: Request) -> Result<Response, WireError> {
            let line = serde_json::to_string(&request).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            if let Some(canned) = &self.answer_everything_with {
                return Ok(serde_json::from_str(&serde_json::to_string(canned).unwrap()).unwrap());
            }
            let response = Server.dispatch(request).await?;
            Ok(serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap())
        }
    }

    fn block_on<F: Future>(fut: F) -> F::Output {
        let mut fut = std::pin::pin!(fut);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[test]
    fn requests_and_responses_round_trip_as_tagged_json() {
        let json = serde_json::to_string(&Request::Echo {
            text: "hi".into(),
            upper: true,
        })
        .unwrap();
        assert_eq!(json, r#"{"echo":{"text":"hi","upper":true}}"#);
        let request: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(request.command(), "echo");
        let response = block_on(Server.dispatch(request)).unwrap();
        assert_eq!(response.command(), "echo");
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"echo":"HI"}"#
        );
        let back: Response = serde_json::from_str(r#"{"ping":7}"#).unwrap();
        assert!(matches!(back, Response::Ping(7)));
    }

    #[test]
    fn dispatch_routes_each_variant_to_its_method() {
        assert!(matches!(
            block_on(Server.dispatch(Request::Ping { n: 1 })),
            Ok(Response::Ping(2))
        ));
        let err = block_on(Server.dispatch(Request::Reset {})).unwrap_err();
        assert_eq!(err.code, "reset_refused");
        assert_eq!(
            serde_json::to_string(&Request::Reset {}).unwrap(),
            r#"{"reset":{}}"#
        );
    }

    #[test]
    fn client_methods_unwrap_their_own_variant_and_refuse_another() {
        let client = Client {
            answer_everything_with: None,
        };
        assert_eq!(block_on(client.echo("hi".into(), true)).unwrap(), "HI");
        assert_eq!(block_on(client.ping(41)).unwrap(), 42);
        assert_eq!(block_on(client.reset()).unwrap_err().code, "reset_refused");

        let crossed = Client {
            answer_everything_with: Some(Response::Ping(0)),
        };
        let err = block_on(crossed.echo("hi".into(), false)).unwrap_err();
        assert_eq!(err.code, "protocol_error");
        assert_eq!(err.message, "echo was answered with a ping response");
    }
}
