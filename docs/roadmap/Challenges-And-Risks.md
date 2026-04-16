# Challenges and Risks

> What could go wrong — and how to mitigate.

---

## Technical Challenges

### 1. SELinux Policy Complexity

**Risk**: Android's SELinux is strict. Every denied access crashes or silently fails.

**Mitigation**:
- Start in `permissive` mode during development
- Use `audit2allow` to generate rules from denials
- Build the policy incrementally (add rules one by one)
- Test in `enforcing` mode before declaring "done"

See [[../knowledge/SELinux-Policy]] for details.

**Severity**: High. This will consume the most unexpected time in [[../roadmap/Phase-6-Android-Deploy|Phase 6]].

### 2. Device-Specific Hardware Variance

**Risk**: evdev device names, framebuffer formats, modem interfaces, and DRM configurations vary across devices.

**Mitigation**:
- Auto-detection at startup (scan, probe, identify)
- Configuration overrides in [[../implementation/peko-config|config.toml]] for manual paths
- Test on 2-3 different devices early
- Abstract device differences behind [[../implementation/peko-hal|HAL]] traits

**Severity**: Medium. Expected and manageable with good abstraction.

### 3. Framebuffer Deprecation

**Risk**: Modern Android devices (API 30+) may not expose `/dev/graphics/fb0`.

**Mitigation**:
- Implement both Framebuffer and [[../knowledge/Screen-Capture|DRM/KMS]] capture
- Fallback to `screencap` binary in hybrid mode
- Test DRM path on modern devices early (e.g., Pixel 6+)

**Severity**: Medium. DRM is more complex but documented.

### 4. Modem Interface Compatibility

**Risk**: Not all modems support classic AT commands. Qualcomm QMI is binary protocol, not text.

**Mitigation**:
- Start with devices known to support AT (MediaTek, older Qualcomm)
- QMI support can be added later via the `libqmi` bindings
- SMS can also be sent via Android's `service call` if framework is running (hybrid mode)

**Severity**: Medium. Limits device compatibility but doesn't block core functionality.

### 5. Vision LLM Accuracy

**Risk**: LLM may misidentify UI elements, tap wrong coordinates, or hallucinate button positions.

**Mitigation**:
- Combine screenshot with UiDump XML when available
- Implement coordinate validation (within screen bounds)
- Consider [[../research/Computer-Use-Agents|Set-of-Mark prompting]] for visual grounding
- Use highest-resolution screenshots the model supports
- Retry logic: take another screenshot after action to verify

**Severity**: High. This is the fundamental challenge in all [[../research/Mobile-Agents|mobile agent research]]. Current SOTA is ~30-67% depending on benchmark.

### 6. Context Window Management

**Risk**: Long tasks with many screenshots can exhaust the context window before compression kicks in.

**Mitigation**:
- Aggressive image downscaling (720p or lower)
- [[../implementation/Context-Compression|Compression]] threshold set conservatively (0.6 instead of 0.7)
- Consider JPEG (much smaller than PNG) for screenshots
- Store screenshots externally, reference by ID instead of inline base64

**Severity**: Medium. Solved by engineering, not research.

### 7. Network Reliability on Mobile

**Risk**: Mobile networks are unreliable. API calls may timeout or fail mid-stream.

**Mitigation**:
- [[../implementation/LLM-Providers|ProviderChain]] with automatic failover
- Exponential backoff retry on transient failures
- Local model fallback (local models via on-device inference)
- Save conversation state to [[../implementation/Session-Persistence|SQLite]] after every iteration

**Severity**: Low-Medium. Standard reliability engineering.

## Safety Risks

### 8. Unintended Actions

**Risk**: Agent sends SMS to wrong number, makes unintended calls, deletes files.

**Mitigation**:
- `is_dangerous()` flag on tools requiring confirmation
- Control socket confirmation workflow
- Path sandboxing on filesystem operations
- Budget limit prevents infinite loops
- All actions logged to session store for audit

**Severity**: High impact but mitigated by design. See [[../implementation/Tool-System]].

### 9. API Key Security

**Risk**: API keys stored in plaintext on device filesystem.

**Mitigation**:
- Store in `/data/peko/` with restrictive permissions (0600)
- SELinux policy restricts access to peko_agent domain only
- Environment variable overrides (don't store in config file)
- Future: integrate with Android Keystore for hardware-backed key storage

**Severity**: Medium. Standard secret management.

### 10. Root Access Security

**Risk**: A root-level process with network access is a high-value target.

**Mitigation**:
- SELinux domain restricts capabilities to what's needed
- No incoming network connections (agent is client-only)
- Control socket has filesystem permissions (0660)
- No remote code execution capability (tools are compiled-in)

**Severity**: Medium. Inherent in the architecture, mitigated by SELinux.

## Project Risks

### 11. Scope Creep

**Risk**: Trying to implement every feature before shipping anything.

**Mitigation**:
- Follow the [[Implementation-Roadmap|phased approach]]
- MVP = screenshot + touch + shell + single LLM provider
- Add telephony, advanced vision, multi-provider after MVP works

### 12. Single-Device Trap

**Risk**: Developing for one device and discovering it doesn't work on others.

**Mitigation**:
- Test on at least 2 devices with different chipsets by Phase 4
- Keep hardware-specific code in [[../implementation/peko-hal|peko-hal]] only
- Use auto-detection + config overrides

## Risk Matrix

| Risk | Likelihood | Impact | Mitigation effort |
|---|---|---|---|
| SELinux complexity | High | High | Medium (iterative) |
| Device variance | High | Medium | Low (abstraction) |
| Framebuffer deprecated | Medium | Medium | Medium (DRM fallback) |
| Modem incompatibility | Medium | Low | Low (not required for MVP) |
| Vision accuracy | High | High | Ongoing research problem |
| Context overflow | Medium | Medium | Low (engineering) |
| Network reliability | Medium | Low | Low (retry + failover) |
| Unintended actions | Low | High | Low (already designed) |

## Related

- [[Possibilities]] — Positive future directions
- [[Implementation-Roadmap]] — Phased approach to manage risk
- [[Device-Requirements]] — Device selection guidance
- [[Testing-Strategy]] — Catching issues early

---

#roadmap #risks #challenges
