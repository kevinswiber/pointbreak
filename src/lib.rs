pub mod crypto;
pub mod documents;
pub mod error;
pub mod git;
pub mod highlight;
pub mod keys;
pub mod model;
pub mod perf;
pub mod session;

mod canonical_hash;
mod storage;

// Developer-run measurement support for the durable store's file backend. Behind
// `cfg(test)` (so its smoke test runs under the test runner) and the `bench`
// feature (so the `store_backend` benchmark crate can reach it); never in a
// release build, so it stays off the published surface.
#[cfg(any(test, feature = "bench"))]
#[doc(hidden)]
pub mod bench_support;

#[cfg(test)]
mod test_tracing;
