@echo off
:: Throwaway pass-through git shim (Windows). Placed first on PATH for one dispatched
:: measurement run: it COUNTS each spawn (one line per invocation in GIT_SHIM_LOG),
:: then execs the real git resolved into GIT_SHIM_REAL. It is never a no-op — fixtures
:: and production shell out to real git, so a stubbed shim would break setup instead of
:: measuring the floor. Per-spawn latency is measured separately by a bare-git loop, not
:: from this wrapper (the wrapper inflates durations).
>> "%GIT_SHIM_LOG%" echo %*
"%GIT_SHIM_REAL%" %*
