# Signing UX

Pointbreak events may carry an optional Ed25519 signature that authenticates the producer facts. This
page orients you across the three signing flows and the verification ladder. Reference material lives
in [cli-reference.md](./cli-reference.md) (the `pointbreak key` family and signing env vars),
[storage-model.md](./storage-model.md) (the allowed-signers format and the user-level key home), and
[ADR-0010](./adr/adr-0010-actor-identity-and-delegation.md) (the decisions).

## Signing never gates a write

This is the load-bearing rule: **a write never fails because of signing.** Whatever goes wrong while
resolving a key — none configured, an unreadable key home, an unsupported algorithm, a malformed
configured key, or `POINTBREAK_SIGNING=off` — degrades to an **unsigned write at exit 0**, with a one-line
advisory diagnostic on stderr. Signing strengthens a write when it can; it never blocks one.

## The verification ladder

A read surface (with a verification policy and the discovered trust set) renders one of:

- **`unsigned`** — no signature: no key was configured, `POINTBREAK_SIGNING=off`, or keygen failed. The
  write still happened.
- **`untrusted_key`** — signed by a key that is not in the repo's `.pointbreak/allowed-signers.json`
  allow-list. Tamper-evident and strictly better than unsigned, but not yet bound to an actor.
- **`valid`** — signed by a key enrolled for that actor (or a self-certifying `did:key` actor whose
  id is its own signer). The signature verifies and binds.

(`invalid` is the fourth status — a signature that fails to verify against its claimed key.)

## Three flows

### Human: `init` then `enroll`

```bash
pointbreak key init --name default          # generate a key, print its did:key
pointbreak key enroll default --actor actor:git-email:alice@example.com
git add .pointbreak/allowed-signers.json && git commit   # the commit is the authorization
```

The human opts in explicitly. Until the enrollment is committed, the human's signed events render
`untrusted_key`; once committed, they render `valid`.

### Agent: auto-keygen on first write

An agent writing under an `actor:agent:*` id needs no setup. The first write silently generates a
passphrase-less per-machine key, signs, and prints a notice with the agent's `did:key` and
`pointbreak key enroll`. A human reviews and commits the allow-list edit to bind the agent. See
[agent-authoring.md](./agent-authoring.md). `POINTBREAK_SIGNING=off` opts out.

### CI: ephemeral self-certifying `did:key`

CI can sign without any enrollment by making the writing actor *be* the signing key:

```bash
pointbreak key init --name ci
export POINTBREAK_ACTOR_ID="$(pointbreak key show ci --did | jq -r .didKey)"
pointbreak capture --sign-key ci   # writer.actorId == signer -> self-certifying
```

Because `writer.actorId` is the signing key's `did:key`, the event omits the top-level `signer` and
verifies `valid` under an empty trust set — no allow-list entry required. The key is ephemeral to the
CI run.

## Reuse your SSH key (ssh-agent)

Already sign your git commits with SSH? Sign your review facts the same way — the same move git made
with `gpg.format=ssh` + `user.signingKey`. Adopt an existing Ed25519 SSH key as an agent-backed signer
with no new key ceremony:

```bash
pointbreak key discover --repo .                               # inspect Git/OpenSSH evidence
pointbreak key use-ssh ~/.ssh/id_ed25519.pub --name default   # adopt; print its did:key
pointbreak key enroll --signer did:key:z6Mk... --actor actor:git-email:alice@example.com
git add .pointbreak/allowed-signers.json && git commit          # the commit is the authorization
```

`pointbreak key discover` reads local Git SSH signing config and OpenSSH allowed-signers files as setup
evidence. Discovery does not authorize keys: review candidate details first, then choose whether to
adopt signing custody with `pointbreak key use-ssh` and/or stage Pointbreak trust with `pointbreak key enroll
--signer`. The positional `use-ssh` argument is either a path to a `*.pub` file or a
`key::ssh-ed25519 AAAA…` literal (git's `user.signingKey` form). Here "agent" is the **ssh-agent**
key custodian — distinct from a coding or reviewing agent (acting software).

**No Ed25519 SSH key yet?** Create one (the same key works for git commit signing):

```bash
ssh-keygen -t ed25519 -C "you@example.com"   # writes ~/.ssh/id_ed25519 and ~/.ssh/id_ed25519.pub
```

**Load it into ssh-agent so signing doesn't prompt every write.** Pointbreak signs *through* the agent,
so the private key must be loaded in it:

```bash
ssh-add ~/.ssh/id_ed25519     # load it (add --apple-use-keychain on macOS to persist across logins)
ssh-add -l                    # verify it is listed
```

If the key is not loaded, `pointbreak key list` reports `agentLoaded: false` and a signed write degrades to
**unsigned, exit 0** (`signing_agent_key_absent`) — never blocking, but not yet `valid`. Load the key
and re-run the write.

**ssh-agent custody.** The private key is **never read**: ssh-agent custodies it, and Pointbreak only
ever ships the DSSE pre-authentication bytes to the agent and unwraps the returned signature. Encrypted
keys, 1Password, and hardware-backed agents all work for free.

**Plain `ssh-ed25519` only.** Other key types are rejected with a clear diagnostic at `use-ssh` time:

- `ed25519-sk` (FIDO/`-sk`) signs a hash + flags + counter construction, never the raw message, so a
  signature from one can **never verify** under the strict Ed25519 path. Hard exclusion.
- RSA and ECDSA are not Ed25519 — rejected, pointing at `pointbreak key init`.
- **No SSHSIG wrapper.** The `DSSEv1 ` PAE prefix already supplies domain separation, so Pointbreak sends
  the bytes to the agent raw (sign flags = 0) and unwraps the SSH-wire `string "ssh-ed25519", string sig`
  response to the 64-byte signature ADR-0004 wants. Cross-protocol confusion with the same key is
  structurally excluded.

**Never gates, even though the agent can fail.** A network-backed signer can fail where a local file
key cannot, so signing-never-gates is kept true by two cooperating mechanisms: an **identities-only
pre-flight** in the resolve layer (connect + confirm the key is loaded — it does **not** sign) and a
**tightly-scoped sign-time degrade** in the write seam. Every failure mode leaves an **unsigned write at
exit 0** with a named diagnostic:

| Agent condition | Diagnostic | Outcome |
| --------------- | ---------- | ------- |
| Agent unavailable (no socket / no pipe / refused) — at pre-flight | `signing_agent_unavailable` | unsigned write, exit 0 |
| Key not loaded / globally locked (lists zero identities) — at pre-flight | `signing_agent_key_absent` | unsigned write, exit 0 |
| Agent refuses or fails the sign (confirmation deny, agent died) — at sign time | `signing_agent_sign_failed` | unsigned write, exit 0 |
| Unsupported key algorithm (`-sk`/RSA/ECDSA) | `signing_key_unsupported_algorithm` | unsigned write, exit 0 |

For an **explicitly selected** agent-backed key (`--sign-key` / `POINTBREAK_SIGNING_KEY`), a pre-flight
failure is terminal — it does not fall through to auto-keygen or the `default` key.

**One prompt per write, not two.** Because the pre-flight is identities-only (it never signs), a
confirmation-constrained or hardware agent (`ssh-add -c`, 1Password, YubiKey) is prompted exactly once
per write — at the real sign. (A probe-sign pre-flight was rejected for exactly this reason: it would
prompt twice.) The will-it-sign question is answered at the real sign, and the sign-time degrade closes
the sub-second window where the agent dies or locks between pre-flight and sign (and a per-key
confirmation deny): the event is left unsigned, still exit 0, with `signing_agent_sign_failed`. The
file-signer path stays strict — its errors still propagate, so a real bug is never masked. There is no
residual.

**Cross-platform.** The transport is platform-abstracted: the `$SSH_AUTH_SOCK` Unix domain socket, and
the Windows named pipe `\\.\pipe\openssh-ssh-agent`.

## Deferred

Key rotation and revocation are named follow-ons, not yet shipped — only the flows above are available
today.
