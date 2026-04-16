# Phase 8: Messaging Gateway

> Talk to the agent from Telegram, Discord, or any platform — not just the web UI.

---

## Why This Matters

The web UI requires a browser and port forwarding. With a messaging gateway,
you can control the agent from your phone via Telegram while the Android device
sits headless on a shelf.

---

## Priority: Telegram First

Telegram is the most practical for an Android device agent:
- Works on any phone/desktop
- Rich media (photos, audio, files)
- Bot API is simple and well-documented
- No additional infrastructure needed

### Architecture

```
┌──────────────┐     HTTPS      ┌───────────────────┐
│ Telegram App │ ◄────────────► │ Telegram Bot API   │
│ (your phone) │                │ api.telegram.org   │
└──────────────┘                └────────┬──────────┘
                                         │ webhook/polling
                                         ▼
                                ┌───────────────────┐
                                │ peko-agent      │
                                │ TelegramGateway    │
                                │                    │
                                │ /msg → run_task()  │
                                │ result → sendMsg() │
                                │ screenshot → photo │
                                └───────────────────┘
```

### Message Flow

```
User (Telegram): "Open Chrome and go to google.com"
  → Bot receives message
  → AgentRuntime.run_task("Open Chrome and go to google.com")
  → Agent takes screenshots, taps, navigates
  → Response text sent back to Telegram
  → Final screenshot sent as photo
```

### Config

```toml
[messaging.telegram]
enabled = true
bot_token = "123456:ABC-DEF..."
allowed_users = [12345678]  # Telegram user IDs
send_screenshots = true
```

### Tasks

- [ ] Add `reqwest`-based Telegram Bot API client
- [ ] Implement long-polling message loop
- [ ] Route incoming messages to AgentRuntime.run_task()
- [ ] Send text responses back
- [ ] Send screenshots as photos
- [ ] User whitelist for security
- [ ] Config: bot_token, allowed_users, send_screenshots
- [ ] Add Telegram config section in web UI

### Future Platforms

After Telegram, add in order of value:
1. **Discord** — similar bot API, community use case
2. **Slack** — enterprise use case
3. **WhatsApp** — via WhatsApp Business API or Baileys
4. **Signal** — via signal-cli
5. **Matrix** — open protocol, self-hosted

---

#roadmap #phase-8 #messaging #telegram
