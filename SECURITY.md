# Security Policy

Peko Agent ships a binary that runs **as root on a user's phone**, holds the
**default-SMS role**, **places calls**, **records voice calls**, and exposes a
local **HTTP API on port 8080**. A security bug in Peko Agent is a security
bug in someone's primary communication device, so please treat vulnerabilities
in this codebase as high-value and report them carefully.

## What counts as a vulnerability

High-severity categories we want reports on:

- **Privilege escalation** — any path that lets a non-root caller reach a
  root-only code path (e.g. a web UI endpoint that executes shell commands
  for unauthenticated callers).
- **Unauthorised SMS / call / recording.** The audit log at
  `/data/peko/sms_sent.log` + `/data/peko/calls.log` must stay truthful; any
  way to make the agent send / dial / record without leaving a trace is a
  vulnerability.
- **Consent-beep bypass** in the call recording pipeline. The two 440 Hz
  tones at the start of every recording are the legal consent signal — if
  you find a code path that records without playing them, it's a bug.
- **Credential leaks.** API keys in logs, in error messages, in `/api/config`
  responses, or in the web UI before masking. The expected surface is
  `first4...last4`; anything revealing more is a defect.
- **RCE / command injection** via any tool input, especially `shell`,
  `filesystem`, `package_manager`, `sms`, and `call`.
- **Cross-session data bleed** — one task's output ending up in another
  user's conversation history.
- **SSRF / request smuggling** via the LLM provider chain (e.g. an attacker-
  controlled model response that steers peko into calling a local service
  on the device's network).
- **SELinux policy regressions** — the `sepolicy.rule` file in the Magisk
  module grants narrow allowances; a PR that widens them inappropriately is
  a vulnerability, not a feature.

Things that are **not** security vulnerabilities:

- Missing TLS on `127.0.0.1:8080`. The web UI is localhost-only by design.
  If you want TLS, wrap it in `adb forward` + a reverse tunnel, or front
  it with a proper service.
- The fact that root on the device reads `config.toml` in plaintext. Anyone
  with root already wins.
- Rate limits feeling too low — that's a config knob, not a bug.
- The `VOICE_CALL` audio source being rejected on some OEM HALs. It's a
  hardware limitation; the pipeline falls back to `VOICE_COMMUNICATION`
  and records the outcome in the metadata.

## How to report

Email **tanuphat.chai@gmail.com** with:

1. **Subject line** starting with `[security]`.
2. **Summary** — one or two sentences.
3. **Affected component + version** — commit hash, binary SHA256 if you have
   it, and whether you're on Magisk or a LineageOS overlay build.
4. **Reproduction** — steps, a PoC script, or a curl invocation.
5. **Impact** — what an attacker can do with it.
6. **Suggested fix** if you have one (optional).

Please **do not** open a public GitHub issue for security bugs. If you
accidentally have, delete it and follow up over email.

## Response timelines

This is a one-maintainer project, so response times are best-effort, but:

- **Acknowledgement**: within 72 hours of your email.
- **Triage + severity assessment**: within 7 days.
- **Fix + release**: varies. Critical RCE / credential-leak class bugs get a
  patched release within 14 days. Lower-severity issues are bundled into
  the next planned release.

You'll get an update at each step. If a week has passed without an
acknowledgement, resend — mail can get lost.

## Coordinated disclosure

Once we've agreed on a fix:

- We'll merge the patch to `main`, cut a release (tag + signed Magisk module),
  and publish a short advisory in `docs/security/` with the CVE if one is
  assigned.
- You get credit in the advisory unless you prefer to stay anonymous.
- We ask for a **30-day embargo** between the fix being released and you
  writing a public post about the bug, to give operators time to upgrade. A
  shorter embargo is fine if the bug is already being exploited in the wild.

## Bug-bounty

There is no paid bug-bounty programme. This is a hobby project under AGPL;
treat responsible disclosure as its own reward. If the fix is invasive and
your report saved real harm, we'll say so in the advisory and on the repo.

## Security posture reminders for users

- Don't expose port 8080 to anything other than `localhost` (or an
  `adb forward`). The web UI has no authentication.
- Treat the device `config.toml` as secret — it contains API keys and,
  optionally, your lockscreen PIN.
- `[calls]` records audio. Consent-beep notwithstanding, check local law
  before enabling it for calls you don't control both sides of.
- The `sms` + `call` tools spend real money. Leave the rate limits on.
