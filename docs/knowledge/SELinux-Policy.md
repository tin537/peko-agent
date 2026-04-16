# SELinux Policy

> Mandatory access control for the Peko Agent domain.

---

## Why SELinux Matters

Android enforces SELinux in **enforcing mode** — every process must have an explicit policy allowing every action it takes. Without a proper policy, Peko Agent gets denied at every syscall, even as root.

SELinux provides security boundaries even for root processes. The agent can access what it needs and nothing more.

## Policy Files

### Type Enforcement (`peko_agent.te`)

```
# Define the domain and executable types
type peko_agent, domain;
type peko_agent_exec, exec_type, file_type, system_file_type;

# Transition: when init executes peko_agent_exec, enter peko_agent domain
init_daemon_domain(peko_agent)

# --- Kernel device access ---

# Input devices (touch injection, key events)
allow peko_agent input_device:chr_file { open read write ioctl };

# Display (screenshots)
allow peko_agent gpu_device:chr_file rw_file_perms;
allow peko_agent graphics_device:chr_file rw_file_perms;

# TTY/serial (modem AT commands)
allow peko_agent tty_device:chr_file rw_file_perms;

# Linux capabilities
allow peko_agent self:capability { net_raw net_admin sys_ptrace };

# --- Network access (for LLM API calls) ---
allow peko_agent self:tcp_socket create_stream_socket_perms;
allow peko_agent self:udp_socket create_socket_perms;
allow peko_agent port:tcp_socket name_connect;

# DNS resolution
allow peko_agent dns_resolve:service_manager find;

# --- Filesystem ---

# Peko data directory
allow peko_agent peko_data_file:dir create_dir_perms;
allow peko_agent peko_data_file:file create_file_perms;

# Read system files (config, binaries)
allow peko_agent system_file:file { read open getattr };

# --- Optional: Binder IPC (for HAL access in hybrid mode) ---
allow peko_agent binder_device:chr_file rw_file_perms;
binder_use(peko_agent)

# --- Logging ---
allow peko_agent logd:unix_dgram_socket sendto;
```

### File Contexts (`file_contexts`)

Labels files so SELinux knows which types apply:

```
# Binary
/system/bin/peko-agent    u:object_r:peko_agent_exec:s0

# Data directory
/data/peko(/.*)?           u:object_r:peko_data_file:s0

# Config
/data/peko/config\.toml    u:object_r:peko_data_file:s0

# Database
/data/peko/state\.db       u:object_r:peko_data_file:s0
```

### Property Contexts (optional)

If using property triggers:

```
sys.peko.     u:object_r:peko_prop:s0
```

## How SELinux Domains Work

```
1. init reads init.rc → launches /system/bin/peko-agent
2. /system/bin/peko-agent has label u:object_r:peko_agent_exec:s0
3. init_daemon_domain() macro creates automatic transition:
   init (u:r:init:s0) executes peko_agent_exec → enters peko_agent domain
4. Process now runs as u:r:peko_agent:s0
5. Every syscall checked against peko_agent.te rules
```

## Common Denial Patterns

When developing, `adb logcat | grep avc` shows denials:

```
avc: denied { read } for name="event2" dev="tmpfs"
  scontext=u:r:peko_agent:s0
  tcontext=u:object_r:input_device:s0
  tclass=chr_file
```

Fix: add `allow peko_agent input_device:chr_file { read };` to the `.te` file.

## Policy Development Workflow

1. Start with a minimal policy (binary + data directory only)
2. Run the binary → collect denials from logcat
3. Use `audit2allow` to generate allow rules from denials
4. Add rules to `.te` file
5. Rebuild and reload policy
6. Repeat until no more denials

```bash
# Collect denials
adb logcat -d | grep peko_agent | grep avc > denials.txt

# Generate allow rules
audit2allow -i denials.txt

# Review and add to peko_agent.te
```

## Security Considerations

Even with root and a permissive policy, SELinux provides defense-in-depth:

| Principle | Implementation |
|---|---|
| Least privilege | Only allow the specific device accesses needed |
| Type isolation | peko_data_file can't be accessed by other domains |
| Network restriction | Only TCP/UDP for API calls, not raw packet injection |
| No execution of other binaries | Unless explicitly allowed (e.g., `screencap`) |

For production, the policy should be as tight as possible. For development, use `permissive` mode to collect all needed permissions first.

## Related

- [[Android-Internals]] — Where SELinux fits in the boot process
- [[../architecture/Boot-Sequence]] — seclabel directive in init.rc
- [[Linux-Kernel-Interfaces]] — What the policy grants access to
- [[../roadmap/Challenges-And-Risks]] — SELinux as a deployment challenge

---

#knowledge #selinux #security #android
