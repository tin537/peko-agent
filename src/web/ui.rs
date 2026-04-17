pub const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en" class="dark">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no">
<title>Peko Agent</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
<script src="https://cdn.tailwindcss.com"></script>
<script>
tailwind.config = {
  darkMode: 'class',
  theme: {
    extend: {
      fontFamily: {
        sans: ['Inter', 'system-ui', '-apple-system', 'sans-serif'],
        mono: ['JetBrains Mono', 'SF Mono', 'Fira Code', 'monospace'],
      },
    }
  }
}
</script>
<style>
  ::-webkit-scrollbar { width: 6px; height: 6px; }
  ::-webkit-scrollbar-track { background: transparent; }
  ::-webkit-scrollbar-thumb { background: rgb(63 63 70 / 0.5); border-radius: 3px; }
  ::-webkit-scrollbar-thumb:hover { background: rgb(82 82 91); }
  @keyframes msg-in {
    from { opacity: 0; transform: translateY(6px); }
    to { opacity: 1; transform: translateY(0); }
  }
  .msg-in { animation: msg-in 0.2s ease-out; }
  *:focus-visible {
    outline: 2px solid rgb(139 92 246 / 0.5);
    outline-offset: 2px;
    border-radius: 4px;
  }
  select {
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%2371717a' stroke-width='2'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 12px center;
    padding-right: 36px;
  }
  textarea { scrollbar-width: thin; scrollbar-color: rgb(63 63 70 / 0.5) transparent; }
</style>
</head>
<body class="bg-zinc-950 text-zinc-100 h-screen overflow-hidden font-sans antialiased">
<div id="app" class="flex flex-col h-full">

  <!-- Header -->
  <header class="h-14 min-h-[3.5rem] flex items-center justify-between px-4 bg-zinc-900/80 backdrop-blur-md border-b border-zinc-800/80 z-10">
    <div class="flex items-center gap-3">
      <button id="sidebarToggle" onclick="toggleSidebar()" class="md:hidden p-1.5 -ml-1 rounded-lg hover:bg-zinc-800 transition-colors" aria-label="Toggle sidebar">
        <svg class="w-5 h-5 text-zinc-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5"/></svg>
      </button>
      <div class="flex items-center gap-2.5">
        <span id="statusDot" class="relative flex h-2.5 w-2.5">
          <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
          <span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span>
        </span>
        <h1 class="text-[15px] font-semibold tracking-tight">
          <span class="text-zinc-100">Peko</span><span class="text-zinc-500 font-normal ml-0.5">Agent</span>
        </h1>
      </div>
      <div class="hidden sm:flex items-center gap-2 ml-1">
        <div class="h-4 w-px bg-zinc-800"></div>
        <span id="modelInfo" class="inline-flex items-center px-2 py-0.5 bg-zinc-800 rounded-md text-[11px] font-mono text-zinc-400 truncate max-w-[200px]">...</span>
      </div>
    </div>
    <div class="flex items-center gap-3">
      <span id="memInfo" class="hidden sm:inline text-[11px] text-zinc-500 font-mono"></span>
      <nav class="flex bg-zinc-800/60 rounded-lg p-0.5 border border-zinc-700/40" role="tablist">
        <button id="tabChat" onclick="showTab('chat')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 bg-violet-600 text-white shadow-sm">Chat</button>
        <button id="tabMonitor" onclick="showTab('monitor')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Monitor</button>
        <button id="tabApps" onclick="showTab('apps')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Apps</button>
        <button id="tabMsgs" onclick="showTab('messages')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Messages</button>
        <button id="tabMemory" onclick="showTab('memory')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Memory</button>
        <button id="tabSkills" onclick="showTab('skills')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Skills</button>
        <button id="tabCfg" onclick="showTab('config')" role="tab" class="px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200">Config</button>
      </nav>
    </div>
  </header>

  <!-- Body -->
  <div class="flex flex-1 overflow-hidden relative">

    <!-- Sidebar overlay (mobile) -->
    <div id="sidebarOverlay" onclick="toggleSidebar()" class="hidden fixed inset-0 bg-black/50 z-10 md:hidden"></div>

    <!-- Sidebar -->
    <aside id="sidebar" class="w-64 min-w-[16rem] bg-zinc-900 border-r border-zinc-800/80 flex-col
      hidden md:flex
      fixed md:relative inset-y-0 left-0 z-20 md:z-auto top-14 md:top-0 bottom-0">
      <div class="flex items-center justify-between px-4 py-3 border-b border-zinc-800/60">
        <span class="text-[11px] font-bold uppercase tracking-widest text-zinc-500">Sessions</span>
        <button onclick="newChat()" class="inline-flex items-center gap-1 px-2.5 py-1 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700/60 rounded-lg text-[11px] font-medium text-zinc-300 transition-colors" title="New session">
          <svg class="w-3 h-3" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path d="M12 5v14m-7-7h14"/></svg>
          New
        </button>
      </div>
      <div id="sessions" class="flex-1 overflow-y-auto">
        <div id="sessionsEmpty" class="flex flex-col items-center justify-center py-12 px-4">
          <svg class="w-8 h-8 text-zinc-700 mb-2" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"/></svg>
          <p class="text-xs text-zinc-600">No sessions yet</p>
        </div>
        <div id="sessionsList"></div>
      </div>
    </aside>

    <!-- Main content -->
    <main class="flex-1 flex flex-col overflow-hidden min-w-0">

      <!-- Chat Panel -->
      <div id="chatPanel" class="flex flex-col flex-1 overflow-hidden">
        <!-- Messages -->
        <div id="msgs" class="flex-1 overflow-y-auto">
          <!-- Empty state -->
          <div id="emptyChat" class="flex flex-col items-center justify-center h-full px-8 text-center select-none">
            <div class="w-14 h-14 rounded-2xl bg-violet-600/10 border border-violet-500/20 flex items-center justify-center mb-5">
              <svg class="w-7 h-7 text-violet-400" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M10.5 1.5H8.25A2.25 2.25 0 006 3.75v16.5a2.25 2.25 0 002.25 2.25h7.5A2.25 2.25 0 0018 20.25V3.75a2.25 2.25 0 00-2.25-2.25H13.5m-3 0V3h3V1.5m-3 0h3m-3 18.75h3"/></svg>
            </div>
            <h2 class="text-lg font-semibold text-zinc-200 mb-1.5">Peko Agent</h2>
            <p class="text-sm text-zinc-500 max-w-sm leading-relaxed">Send a task to control the Android device. The agent will see the screen, tap, type, and navigate to complete your request.</p>
          </div>
          <!-- Messages list -->
          <div id="msgsList" class="hidden px-4 py-6 space-y-4 max-w-4xl mx-auto w-full"></div>
        </div>

        <!-- Input bar -->
        <div class="border-t border-zinc-800/80 bg-zinc-900/60 backdrop-blur-sm px-4 py-3">
          <div class="max-w-4xl mx-auto">
            <div class="flex gap-3 items-end">
              <div class="flex-1">
                <textarea id="inp" rows="1" placeholder="Enter a task..."
                  class="w-full bg-zinc-800/80 border border-zinc-700/60 rounded-xl px-4 py-3 text-sm text-zinc-100 placeholder-zinc-500 resize-none outline-none focus:border-violet-500/60 focus:ring-1 focus:ring-violet-500/20 transition-all min-h-[44px] max-h-[160px] leading-relaxed"
                  onkeydown="handleKey(event)" oninput="autoResize(this)"></textarea>
              </div>
              <button id="sendBtn" onclick="send()"
                class="h-[44px] px-5 bg-violet-600 hover:bg-violet-500 active:bg-violet-700 text-white rounded-xl text-sm font-semibold transition-all duration-150 disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-2 shadow-sm shadow-violet-600/20 shrink-0">
                Send
                <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.5 10.5L12 3m0 0l7.5 7.5M12 3v18"/></svg>
              </button>
              <button id="stopBtn" onclick="stop()"
                class="hidden h-[44px] px-5 bg-red-600 hover:bg-red-500 active:bg-red-700 text-white rounded-xl text-sm font-semibold transition-all duration-150 items-center gap-2 shrink-0">
                <svg class="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>
                Stop
              </button>
            </div>
            <p class="text-[10px] text-zinc-600 mt-1.5 ml-1">Press Enter to send &middot; Shift+Enter for new line</p>
          </div>
        </div>
      </div>

      <!-- Config Panel -->
      <div id="cfgPanel" class="hidden">
        <div class="max-w-2xl mx-auto px-6 py-8 space-y-8">

          <!-- LLM Provider -->
          <section>
            <div class="flex items-center gap-2 mb-4">
              <svg class="w-4 h-4 text-violet-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.75 3.104v5.714a2.25 2.25 0 01-.659 1.591L5 14.5M9.75 3.104c-.251.023-.501.05-.75.082m.75-.082a24.301 24.301 0 014.5 0m0 0v5.714c0 .597.237 1.17.659 1.591L19.8 15.3M14.25 3.104c.251.023.501.05.75.082M19.8 15.3l-1.57.393A9.065 9.065 0 0112 15a9.065 9.065 0 00-6.23.693L5 14.5m14.8.8l1.402 1.402c1.232 1.232.65 3.318-1.067 3.611A48.309 48.309 0 0112 21c-2.773 0-5.491-.235-8.135-.687-1.718-.293-2.3-2.379-1.067-3.61L5 14.5"/></svg>
              <h3 class="text-xs font-bold uppercase tracking-wider text-violet-400">LLM Provider</h3>
            </div>
            <div class="bg-zinc-900/80 rounded-xl border border-zinc-800/80 p-5 space-y-4">
              <div>
                <label for="cProv" class="block text-xs font-medium text-zinc-400 mb-1.5">Provider</label>
                <select id="cProv" onchange="provChanged()" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors cursor-pointer">
                  <option value="local">OpenAI-Compatible / Custom</option>
                  <option value="openrouter">OpenRouter</option>
                  <option value="anthropic">Anthropic</option>
                </select>
              </div>
              <div>
                <label for="cKey" class="block text-xs font-medium text-zinc-400 mb-1.5">API Key</label>
                <input type="password" id="cKey" placeholder="sk-..." class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors placeholder-zinc-600">
              </div>
              <div>
                <label for="cModel" class="block text-xs font-medium text-zinc-400 mb-1.5">Model</label>
                <input id="cModel" placeholder="e.g. mimo-v2-omni" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors placeholder-zinc-600">
              </div>
              <div>
                <label for="cUrl" class="block text-xs font-medium text-zinc-400 mb-1.5">Base URL</label>
                <input id="cUrl" placeholder="https://api.example.com/v1" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors placeholder-zinc-600">
              </div>
              <div>
                <label for="cMaxTok" class="block text-xs font-medium text-zinc-400 mb-1.5">Max Tokens</label>
                <input type="number" id="cMaxTok" value="4096" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors">
              </div>
            </div>
          </section>

          <!-- Agent -->
          <section>
            <div class="flex items-center gap-2 mb-4">
              <svg class="w-4 h-4 text-violet-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.324.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 011.37.49l1.296 2.247a1.125 1.125 0 01-.26 1.431l-1.003.827c-.293.24-.438.613-.431.992a6.759 6.759 0 010 .255c-.007.378.138.75.43.99l1.005.828c.424.35.534.954.26 1.43l-1.298 2.247a1.125 1.125 0 01-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.57 6.57 0 01-.22.128c-.331.183-.581.495-.644.869l-.213 1.28c-.09.543-.56.941-1.11.941h-2.594c-.55 0-1.02-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 01-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 01-1.369-.49l-1.297-2.247a1.125 1.125 0 01.26-1.431l1.004-.827c.292-.24.437-.613.43-.992a6.932 6.932 0 010-.255c.007-.378-.138-.75-.43-.99l-1.004-.828a1.125 1.125 0 01-.26-1.43l1.297-2.247a1.125 1.125 0 011.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.087.22-.128.332-.183.582-.495.644-.869l.214-1.281z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/></svg>
              <h3 class="text-xs font-bold uppercase tracking-wider text-violet-400">Agent</h3>
            </div>
            <div class="bg-zinc-900/80 rounded-xl border border-zinc-800/80 p-5">
              <div class="grid grid-cols-2 gap-4">
                <div>
                  <label for="cIter" class="block text-xs font-medium text-zinc-400 mb-1.5">Max Iterations</label>
                  <input type="number" id="cIter" value="50" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors">
                </div>
                <div>
                  <label for="cCtx" class="block text-xs font-medium text-zinc-400 mb-1.5">Context Window</label>
                  <input type="number" id="cCtx" value="200000" class="w-full bg-zinc-800 border border-zinc-700/60 rounded-lg px-3 py-2.5 text-sm text-zinc-200 outline-none focus:border-violet-500/60 transition-colors">
                </div>
              </div>
            </div>
          </section>

          <!-- Tools -->
          <section>
            <div class="flex items-center gap-2 mb-4">
              <svg class="w-4 h-4 text-violet-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M11.42 15.17l-5.384 5.384a2.025 2.025 0 01-2.864-2.864l5.384-5.384m2.864 2.864L18 21.75M12.75 3.75a4.5 4.5 0 00-4.5 4.5v2.25c0 .621-.504 1.125-1.125 1.125H4.5m11.25-6.75a4.5 4.5 0 014.5 4.5v2.25c0 .621.504 1.125 1.125 1.125H21"/></svg>
              <h3 class="text-xs font-bold uppercase tracking-wider text-violet-400">Tools</h3>
            </div>
            <div class="bg-zinc-900/80 rounded-xl border border-zinc-800/80 p-5">
              <div class="grid grid-cols-2 sm:grid-cols-3 gap-3">
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tShell" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Shell</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tFs" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Filesystem</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tSs" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Screenshot</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tTouch" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Touch</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tKey" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Key Events</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tText" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Text Input</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tUi" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">UI Dump</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tSms" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">SMS</span>
                </label>
                <label class="flex items-center gap-2.5 px-3 py-2.5 rounded-lg bg-zinc-800/50 border border-zinc-700/30 cursor-pointer hover:border-zinc-600/50 transition-colors">
                  <input type="checkbox" id="tCall" checked class="w-4 h-4 rounded accent-violet-500">
                  <span class="text-sm text-zinc-300">Call</span>
                </label>
              </div>
            </div>
          </section>

          <!-- Save -->
          <div class="flex items-center gap-4 pt-2">
            <button onclick="saveCfg()" class="px-6 py-2.5 bg-violet-600 hover:bg-violet-500 active:bg-violet-700 text-white rounded-xl text-sm font-semibold transition-colors shadow-sm shadow-violet-600/20">
              Save Changes
            </button>
            <span id="cfgSaved" class="hidden text-xs font-medium text-emerald-400 flex items-center gap-1.5">
              <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>
              Saved! Changes apply on next task.
            </span>
          </div>

          <!-- SOUL.md Editor -->
          <div class="mt-8 pt-6 border-t border-zinc-800/60">
            <div class="flex items-center justify-between mb-3">
              <div>
                <h3 class="text-xs font-bold text-zinc-300 uppercase tracking-wider">SOUL.md — Agent Personality</h3>
                <p class="text-[10px] text-zinc-600 mt-0.5">Customize how the agent thinks, speaks, and behaves</p>
              </div>
              <div class="flex items-center gap-2">
                <button onclick="saveSoul()" class="px-4 py-1.5 bg-violet-600 hover:bg-violet-500 text-white rounded-lg text-xs font-semibold transition-colors">Save SOUL</button>
                <span id="soulSaved" class="hidden text-xs text-emerald-400">Saved!</span>
              </div>
            </div>
            <textarea id="soulEditor" rows="12" class="w-full bg-zinc-950 border border-zinc-800 rounded-xl px-4 py-3 text-xs font-mono text-zinc-300 leading-relaxed outline-none focus:border-violet-500 resize-y" placeholder="Loading SOUL.md..."></textarea>
          </div>

        </div>
      </div>

      <!-- Device Panel (was Monitor) -->
      <div id="monitorPanel" class="hidden flex-1 overflow-y-auto p-4">
        <div class="max-w-5xl mx-auto space-y-4">
          <!-- Device Identity -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <h2 class="text-[10px] text-zinc-500 uppercase mb-3 font-bold tracking-wider">Device Profile</h2>
            <div id="devProfile" class="grid grid-cols-2 md:grid-cols-3 gap-3 text-sm">
              <div><span class="text-zinc-500 text-[10px]">Model</span><p id="dpModel" class="text-zinc-200 font-medium">--</p></div>
              <div><span class="text-zinc-500 text-[10px]">Android</span><p id="dpAndroid" class="text-zinc-200 font-medium">--</p></div>
              <div><span class="text-zinc-500 text-[10px]">Architecture</span><p id="dpArch" class="text-zinc-200 font-mono">--</p></div>
              <div><span class="text-zinc-500 text-[10px]">Screen</span><p id="dpScreen" class="text-zinc-200 font-mono">--</p></div>
              <div><span class="text-zinc-500 text-[10px]">RAM</span><p id="dpRam" class="text-zinc-200 font-mono">--</p></div>
              <div><span class="text-zinc-500 text-[10px]">SELinux</span><p id="dpSE" class="text-zinc-200">--</p></div>
            </div>
          </div>
          <!-- Available Tools -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <h2 class="text-[10px] text-zinc-500 uppercase mb-3 font-bold tracking-wider">Agent Tools</h2>
            <div id="devTools" class="flex flex-wrap gap-2"></div>
          </div>
          <!-- Hardware Capabilities -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <h2 class="text-[10px] text-zinc-500 uppercase mb-3 font-bold tracking-wider">Hardware</h2>
            <div id="devHw" class="flex flex-wrap gap-2"></div>
          </div>
          <!-- Live Resources -->
          <div class="flex items-center justify-between mb-1 mt-2">
            <h2 class="text-[10px] text-zinc-500 uppercase font-bold tracking-wider">Live Resources</h2>
          </div>
          <div id="statsGrid" class="grid grid-cols-2 md:grid-cols-4 gap-3">
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">CPU</p><p id="sCpu" class="text-xl font-bold text-zinc-200">--</p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Memory</p><p id="sMem" class="text-xl font-bold text-zinc-200">--</p><p id="sMemDetail" class="text-[10px] text-zinc-500 mt-1"></p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Battery</p><p id="sBat" class="text-xl font-bold text-zinc-200">--</p><p id="sBatDetail" class="text-[10px] text-zinc-500 mt-1"></p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Disk</p><p id="sDisk" class="text-xl font-bold text-zinc-200">--</p><p id="sDiskDetail" class="text-[10px] text-zinc-500 mt-1"></p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Network</p><p id="sNet" class="text-sm font-mono text-zinc-200">--</p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Uptime</p><p id="sUp" class="text-xl font-bold text-zinc-200">--</p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Peko RSS</p><p id="sHrss" class="text-xl font-bold text-emerald-400">--</p></div>
            <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4"><p class="text-[10px] text-zinc-500 uppercase mb-1">Load Avg</p><p id="sLoad" class="text-sm font-mono text-zinc-200">--</p></div>
          </div>
          <!-- Processes -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <p class="text-[10px] text-zinc-500 uppercase mb-3">Top Processes</p>
            <div id="sProcs" class="font-mono text-[11px] text-zinc-400 space-y-1"></div>
          </div>
          <!-- Logs -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <div class="flex justify-between items-center mb-3">
              <p class="text-[10px] text-zinc-500 uppercase">Live Logs (logcat)</p>
              <button onclick="toggleLogs()" id="logToggle" class="text-[10px] px-2 py-1 bg-emerald-900/40 border border-emerald-800/30 rounded text-emerald-400">Start</button>
            </div>
            <div id="logBox" class="h-64 overflow-y-auto font-mono text-[10px] text-zinc-500 bg-zinc-950 rounded-lg p-2 whitespace-pre-wrap"></div>
          </div>
        </div>
      </div>

      <!-- Apps Panel -->
      <div id="appsPanel" class="hidden flex-1 overflow-y-auto p-4">
        <div class="max-w-4xl mx-auto">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-sm font-bold text-zinc-300 uppercase tracking-wider">Applications</h2>
            <div class="flex gap-2 items-center">
              <!-- Filter tabs -->
              <div class="flex bg-zinc-800/60 rounded-lg p-0.5 border border-zinc-700/40">
                <button onclick="setAppFilter('user')" id="afUser" class="px-2.5 py-1 rounded-md text-[10px] font-semibold bg-violet-600 text-white">User</button>
                <button onclick="setAppFilter('system')" id="afSystem" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">System</button>
                <button onclick="setAppFilter('all')" id="afAll" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">All</button>
              </div>
              <input id="appSearch" type="text" placeholder="Search..." oninput="filterApps()" class="px-3 py-1.5 bg-zinc-800 border border-zinc-700/50 rounded-lg text-xs text-zinc-300 w-40 outline-none focus:border-violet-500">
            </div>
          </div>
          <div id="appsCount" class="text-[10px] text-zinc-600 mb-2"></div>
          <div id="appsList" class="space-y-1"></div>
        </div>
      </div>

      <!-- Messages Panel -->
      <div id="messagesPanel" class="hidden flex-1 overflow-y-auto p-4">
        <div class="max-w-4xl mx-auto space-y-4">
          <div class="flex items-center justify-between mb-2">
            <h2 class="text-sm font-bold text-zinc-300 uppercase tracking-wider">SMS & Notifications</h2>
            <span id="msgStreamBtn" class="text-[10px] px-3 py-1 bg-emerald-900/40 border border-emerald-800/30 rounded-lg text-emerald-400">Connecting...</span>
          </div>
          <!-- Notifications -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <p class="text-[10px] text-zinc-500 uppercase mb-3">Notifications</p>
            <div id="notifList" class="space-y-2 text-sm text-zinc-400">
              <p class="text-zinc-600 text-xs">Loading notifications...</p>
            </div>
          </div>
          <!-- SMS -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <p class="text-[10px] text-zinc-500 uppercase mb-3">SMS Inbox</p>
            <div id="smsList" class="space-y-2 text-sm text-zinc-400">
              <p class="text-zinc-600 text-xs">Loading messages...</p>
            </div>
          </div>
          <!-- Live events -->
          <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-4">
            <p class="text-[10px] text-zinc-500 uppercase mb-3">Live Events</p>
            <div id="msgEvents" class="h-48 overflow-y-auto font-mono text-[10px] text-zinc-500 bg-zinc-950 rounded-lg p-2"></div>
          </div>
        </div>
      </div>

      <!-- Memory Panel -->
      <div id="memoryPanel" class="hidden flex-1 overflow-y-auto p-4">
        <div class="max-w-4xl mx-auto space-y-4">
          <div class="flex items-center justify-between mb-2">
            <h2 class="text-sm font-bold text-zinc-300 uppercase tracking-wider">Agent Memory</h2>
            <div class="flex gap-2 items-center">
              <span id="memCount" class="text-[10px] text-zinc-500"></span>
              <input id="memSearch" type="text" placeholder="Search memories..." oninput="searchMemories()" class="px-3 py-1.5 bg-zinc-800 border border-zinc-700/50 rounded-lg text-xs text-zinc-300 w-44 outline-none focus:border-violet-500">
            </div>
          </div>
          <!-- Category filters -->
          <div class="flex gap-1.5 flex-wrap">
            <button onclick="filterMem('all')" id="mfAll" class="px-2.5 py-1 rounded-md text-[10px] font-semibold bg-violet-600 text-white">All</button>
            <button onclick="filterMem('fact')" id="mfFact" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">Facts</button>
            <button onclick="filterMem('preference')" id="mfPref" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">Preferences</button>
            <button onclick="filterMem('procedure')" id="mfProc" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">Procedures</button>
            <button onclick="filterMem('observation')" id="mfObs" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">Observations</button>
            <button onclick="filterMem('skill')" id="mfSkill" class="px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200">Skills</button>
          </div>
          <div id="memList" class="space-y-2">
            <p class="text-zinc-600 text-xs text-center py-8">Loading memories...</p>
          </div>
        </div>
      </div>

      <!-- Skills Panel -->
      <div id="skillsPanel" class="hidden flex-1 overflow-y-auto p-4">
        <div class="max-w-4xl mx-auto space-y-4">
          <div class="flex items-center justify-between mb-2">
            <h2 class="text-sm font-bold text-zinc-300 uppercase tracking-wider">Learned Skills</h2>
            <span id="skillCount" class="text-[10px] text-zinc-500"></span>
          </div>
          <div id="skillList" class="space-y-3">
            <p class="text-zinc-600 text-xs text-center py-8">Loading skills...</p>
          </div>
        </div>
      </div>

    </main>
  </div>

  <!-- AGPL §13 source offer — required for network deployments -->
  <footer class="text-[10px] text-zinc-600 px-4 py-2 border-t border-zinc-800/60 flex justify-between items-center font-mono">
    <span>Peko Agent · AGPL-3.0-or-later</span>
    <span>
      <a href="/source" class="text-zinc-500 hover:text-zinc-300 underline-offset-2 hover:underline">get source</a>
      <span class="mx-1.5 text-zinc-700">·</span>
      <a href="/licenses" class="text-zinc-500 hover:text-zinc-300 underline-offset-2 hover:underline">third-party licenses</a>
    </span>
  </footer>
</div>

<script>
const API = window.location.origin;
let busy = false;
let activeSessionId = null;
let sidebarOpen = false;

/* ── Tabs ── */
function showTab(tab) {
  const panels = {chat:'chatPanel',config:'cfgPanel',monitor:'monitorPanel',apps:'appsPanel',messages:'messagesPanel',memory:'memoryPanel',skills:'skillsPanel'};
  const tabs = {chat:'tabChat',config:'tabCfg',monitor:'tabMonitor',apps:'tabApps',messages:'tabMsgs',memory:'tabMemory',skills:'tabSkills'};
  const onClass = 'px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 bg-violet-600 text-white shadow-sm';
  const offClass = 'px-3.5 py-1.5 rounded-md text-xs font-semibold transition-all duration-200 text-zinc-400 hover:text-zinc-200';

  for (var k in panels) {
    var p = document.getElementById(panels[k]);
    var t = document.getElementById(tabs[k]);
    if (k === tab) {
      p.className = k === 'chat' ? 'flex flex-col flex-1 overflow-hidden' : 'flex-1 overflow-y-auto p-4';
      p.style.display = '';
      if (t) t.className = onClass;
    } else {
      p.className = 'hidden';
      if (t) t.className = offClass;
    }
  }
  if (tab === 'config') loadCfg();
  if (tab === 'monitor') { refreshStats(); startMonitorAutoRefresh(); }
  if (tab === 'apps') loadApps();
  if (tab === 'messages') { if (!msgES) startMsgStream(); }
  if (tab === 'memory') loadMemories();
  if (tab === 'skills') loadSkills();
  // Stop streams when leaving tabs
  if (tab !== 'monitor') stopMonitorAutoRefresh();
  if (tab !== 'messages' && msgES) { msgES.close(); msgES = null; }
  if (tab !== 'monitor' && logES) { toggleLogs(); }
}

/* ── Sidebar ── */
function toggleSidebar() {
  const sidebar = document.getElementById('sidebar');
  const overlay = document.getElementById('sidebarOverlay');
  sidebarOpen = !sidebarOpen;
  if (sidebarOpen) {
    sidebar.classList.remove('hidden');
    sidebar.classList.add('flex');
    overlay.classList.remove('hidden');
  } else {
    sidebar.classList.add('hidden');
    sidebar.classList.remove('flex');
    overlay.classList.add('hidden');
  }
}

/* ── Chat messages ── */
function showMsgsList() {
  document.getElementById('emptyChat').classList.add('hidden');
  const list = document.getElementById('msgsList');
  list.classList.remove('hidden');
  return list;
}

function addMsg(type, content) {
  const list = showMsgsList();
  const el = document.createElement('div');
  el.className = 'msg-in';

  if (type === 'user') {
    el.innerHTML =
      '<div class="flex justify-end">' +
        '<div class="max-w-[75%] bg-violet-600 text-white px-4 py-3 rounded-2xl rounded-br-sm text-sm leading-relaxed shadow-sm whitespace-pre-wrap break-words">' +
          content +
        '</div>' +
      '</div>';
  } else if (type === 'assistant') {
    el.innerHTML =
      '<div class="flex justify-start gap-3">' +
        '<div class="w-7 h-7 rounded-lg bg-zinc-800 border border-zinc-700/50 flex items-center justify-center flex-shrink-0 mt-0.5">' +
          '<svg class="w-3.5 h-3.5 text-violet-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>' +
        '</div>' +
        '<div class="max-w-[75%] bg-zinc-800/60 border border-zinc-700/40 px-4 py-3 rounded-2xl rounded-bl-sm text-sm leading-relaxed text-zinc-200 whitespace-pre-wrap break-words" data-text>' +
          content +
        '</div>' +
      '</div>';
  } else if (type === 'tool') {
    el.innerHTML =
      '<div class="flex justify-start gap-3">' +
        '<div class="w-7 h-7 rounded-lg bg-emerald-900/40 border border-emerald-800/30 flex items-center justify-center flex-shrink-0 mt-0.5">' +
          '<svg class="w-3.5 h-3.5 text-emerald-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6.75 7.5l3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0021 18V6a2.25 2.25 0 00-2.25-2.25H5.25A2.25 2.25 0 003 6v12a2.25 2.25 0 002.25 2.25z"/></svg>' +
        '</div>' +
        '<div class="max-w-[85%] bg-emerald-950/20 border border-emerald-800/20 px-4 py-3 rounded-xl overflow-hidden">' +
          content +
        '</div>' +
      '</div>';
  } else if (type === 'error') {
    el.innerHTML =
      '<div class="flex justify-start gap-3">' +
        '<div class="w-7 h-7 rounded-lg bg-red-900/40 border border-red-800/30 flex items-center justify-center flex-shrink-0 mt-0.5">' +
          '<svg class="w-3.5 h-3.5 text-red-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"/></svg>' +
        '</div>' +
        '<div class="max-w-[75%] bg-red-950/20 border border-red-800/20 px-4 py-3 rounded-xl text-sm text-red-300 leading-relaxed whitespace-pre-wrap break-words">' +
          content +
        '</div>' +
      '</div>';
  } else if (type === 'thinking') {
    el.innerHTML =
      '<div class="flex justify-start gap-3">' +
        '<div class="w-7 h-7 flex-shrink-0"></div>' +
        '<div class="max-w-[75%] border border-zinc-700/30 border-dashed px-4 py-2.5 rounded-xl text-xs text-zinc-500 italic leading-relaxed whitespace-pre-wrap break-words">' +
          content +
        '</div>' +
      '</div>';
  }

  list.appendChild(el);
  scrollBottom();
  return el;
}

function addIterBadge(iterations, sessionId) {
  const list = showMsgsList();
  const el = document.createElement('div');
  el.className = 'msg-in flex justify-center py-2';
  el.innerHTML =
    '<div class="inline-flex items-center gap-2 px-3 py-1.5 bg-zinc-800/50 border border-zinc-700/40 rounded-full">' +
      '<svg class="w-3 h-3 text-emerald-500" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/></svg>' +
      '<span class="text-[11px] font-medium text-zinc-400">' + iterations + ' iteration' + (iterations !== 1 ? 's' : '') + '</span>' +
      '<span class="text-[10px] font-mono text-zinc-600">' + ((sessionId || '').slice(0, 8)) + '</span>' +
    '</div>';
  list.appendChild(el);
  scrollBottom();
}

function scrollBottom() {
  const el = document.getElementById('msgs');
  requestAnimationFrame(function() { el.scrollTop = el.scrollHeight; });
}

function showTyping(show) {
  var el = document.getElementById('_typing');
  if (show && !el) {
    var list = showMsgsList();
    el = document.createElement('div');
    el.id = '_typing';
    el.className = 'msg-in flex justify-start gap-3';
    el.innerHTML =
      '<div class="w-7 h-7 rounded-lg bg-zinc-800 border border-zinc-700/50 flex items-center justify-center flex-shrink-0">' +
        '<svg class="w-3.5 h-3.5 text-violet-400" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>' +
      '</div>' +
      '<div class="bg-zinc-800/60 border border-zinc-700/40 px-5 py-3.5 rounded-2xl rounded-bl-sm">' +
        '<div class="flex items-center gap-1.5">' +
          '<span class="text-xs text-zinc-500 mr-1">Working</span>' +
          '<span class="w-1.5 h-1.5 bg-zinc-500 rounded-full animate-bounce" style="animation-delay:0ms"></span>' +
          '<span class="w-1.5 h-1.5 bg-zinc-500 rounded-full animate-bounce" style="animation-delay:150ms"></span>' +
          '<span class="w-1.5 h-1.5 bg-zinc-500 rounded-full animate-bounce" style="animation-delay:300ms"></span>' +
        '</div>' +
      '</div>';
    list.appendChild(el);
    scrollBottom();
  } else if (!show && el) {
    el.remove();
  }
}

function setBusy(b) {
  busy = b;
  document.getElementById('sendBtn').style.display = b ? 'none' : 'flex';
  document.getElementById('stopBtn').style.display = b ? 'flex' : 'none';
  document.getElementById('inp').disabled = b;
  if (!b) document.getElementById('inp').focus();
}

function handleKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
}

function autoResize(el) {
  el.style.height = 'auto';
  el.style.height = Math.min(el.scrollHeight, 160) + 'px';
}

/* ── Send / Stop ── */
async function send() {
  var inp = document.getElementById('inp');
  var text = inp.value.trim();
  if (!text || busy) return;
  inp.value = '';
  inp.style.height = 'auto';

  addMsg('user', esc(text));
  setBusy(true);
  showTyping(true);

  try {
    var res = await fetch(API + '/api/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ input: text, session_id: activeSessionId || undefined })
    });

    showTyping(false);
    if (!res.ok) { addMsg('error', 'HTTP ' + res.status + ': ' + (await res.text())); setBusy(false); return; }

    var reader = res.body.getReader();
    function processSSE(ev) {
      switch (ev.type) {
        case 'text_delta':
          if (!cur) {
            cur = addMsg('assistant', '');
            cur._textEl = cur.querySelector('[data-text]');
          }
          if (cur._textEl) cur._textEl.textContent += ev.text;
          scrollBottom();
          break;
        case 'thinking':
          addMsg('thinking', esc(ev.text));
          break;
        case 'tool_start':
          cur = null;
          showTyping(true);
          break;
        case 'tool_result':
          showTyping(false);
          var toolHtml = '<div class="text-emerald-400 text-[11px] font-semibold font-mono mb-1.5">' + esc(ev.name || 'tool') + '</div>';
          if (ev.image) {
            var imgSrc = ev.image.startsWith('data:') ? ev.image : (API + ev.image);
            toolHtml += '<img src="' + escAttr(imgSrc) + '" class="rounded-lg max-w-full max-h-80 mb-2 border border-zinc-700/30" alt="screenshot" loading="lazy">';
          }
          toolHtml += '<pre class="text-zinc-300 font-mono text-xs whitespace-pre-wrap break-all leading-relaxed">' + esc(ev.content || '') + '</pre>';
          addMsg('tool', toolHtml);
          break;
        case 'done':
          cur = null;
          // Set active session for continuation
          if (ev.session_id) activeSessionId = ev.session_id;
          addIterBadge(ev.iterations, ev.session_id);
          break;
        case 'error':
          addMsg('error', esc(ev.message || 'Unknown error'));
          break;
      }
    }

    var dec = new TextDecoder();
    var cur = null, buf = '';

    while (true) {
      var chunk = await reader.read();
      if (chunk.done) {
        // Process any remaining buffer
        if (buf.trim()) {
          buf.split('\n').forEach(function(line) {
            if (!line.startsWith('data: ')) return;
            var d = line.slice(6);
            if (d === '[DONE]') return;
            try { processSSE(JSON.parse(d)); } catch(e) {}
          });
        }
        break;
      }
      buf += dec.decode(chunk.value, { stream: true });
      var lines = buf.split('\n');
      buf = lines.pop() || '';

      for (var i = 0; i < lines.length; i++) {
        var line = lines[i];
        if (!line.startsWith('data: ')) continue;
        var d = line.slice(6);
        if (d === '[DONE]') continue;
        try { processSSE(JSON.parse(d)); } catch (parseErr) {}
      }
    }
  } catch (e) {
    showTyping(false);
    addMsg('error', 'Connection error: ' + e.message);
  }
  setBusy(false);
  loadSessions();
}

async function stop() {
  try { await fetch(API + '/api/interrupt', { method: 'POST' }); } catch (e) {}
}

/* ── Sessions ── */
var statusColors = {
  running: 'bg-blue-900/50 text-blue-400 border border-blue-800/30',
  completed: 'bg-emerald-900/50 text-emerald-400 border border-emerald-800/30',
  interrupted: 'bg-amber-900/50 text-amber-400 border border-amber-800/30'
};

async function loadSessions() {
  try {
    var r = await fetch(API + '/api/sessions');
    var list = await r.json();
    var emptyEl = document.getElementById('sessionsEmpty');
    var listEl = document.getElementById('sessionsList');

    if (!list.length) {
      emptyEl.classList.remove('hidden');
      listEl.innerHTML = '';
      return;
    }
    emptyEl.classList.add('hidden');

    listEl.innerHTML = list.map(function(s) {
      var isActive = s.id === activeSessionId;
      var colors = statusColors[s.status] || 'bg-zinc-800 text-zinc-400 border border-zinc-700/30';
      var time = (s.started_at || '').slice(11, 16);
      return '<div class="group px-3 py-3 cursor-pointer border-b border-zinc-800/50 transition-colors ' +
        (isActive ? 'bg-violet-600/10 border-l-2 border-l-violet-500' : 'hover:bg-zinc-800/30') +
        '" onclick="loadSession(\'' + escAttr(s.id) + '\')">' +
        '<div class="flex items-start justify-between gap-2">' +
          '<p class="text-sm text-zinc-300 truncate flex-1 leading-snug">' + esc(s.task) + '</p>' +
          '<button class="opacity-0 group-hover:opacity-100 p-0.5 text-zinc-600 hover:text-red-400 transition-all shrink-0" ' +
            'onclick="event.stopPropagation();delSession(\'' + escAttr(s.id) + '\')" title="Delete">' +
            '<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>' +
          '</button>' +
        '</div>' +
        '<div class="flex items-center gap-2 mt-1.5">' +
          '<span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium ' + colors + '">' + s.status + '</span>' +
          '<span class="text-[11px] text-zinc-600">' + s.iterations + ' iter</span>' +
          (time ? '<span class="text-[11px] text-zinc-600">' + time + '</span>' : '') +
        '</div>' +
      '</div>';
    }).join('');
  } catch (e) {}
}

async function loadSession(id) {
  activeSessionId = id;
  showTab('chat');

  var msgsList = document.getElementById('msgsList');
  document.getElementById('emptyChat').classList.add('hidden');
  msgsList.classList.remove('hidden');
  msgsList.innerHTML = '<div class="flex justify-center py-12"><div class="text-xs text-zinc-500">Loading session...</div></div>';

  try {
    var r = await fetch(API + '/api/sessions/' + id);
    var messages = await r.json();
    msgsList.innerHTML = '';

    if (Array.isArray(messages)) {
      for (var i = 0; i < messages.length; i++) {
        var m = messages[i];
        if (m.role === 'user') {
          addMsg('user', esc(m.content));
        } else if (m.role === 'assistant') {
          if (m.content) addMsg('assistant', esc(m.content));
          if (m.tool_args) {
            try {
              var calls = JSON.parse(m.tool_args);
              for (var j = 0; j < calls.length; j++) {
                var tc = calls[j];
                addMsg('tool',
                  '<div class="text-emerald-400 text-[11px] font-semibold font-mono mb-1.5">' + esc(tc.name || 'tool') + '</div>' +
                  '<pre class="text-zinc-300 font-mono text-xs whitespace-pre-wrap break-all leading-relaxed">' + esc(JSON.stringify(tc.input || {}, null, 2)) + '</pre>'
                );
              }
            } catch (e) {}
          }
        } else if (m.role === 'tool_result') {
          if (m.is_error) {
            addMsg('error', esc((m.tool_name || 'tool') + ': ' + m.content));
          } else {
            addMsg('tool',
              '<div class="text-emerald-400 text-[11px] font-semibold font-mono mb-1.5">' + esc(m.tool_name || 'tool') + '</div>' +
              '<pre class="text-zinc-300 font-mono text-xs whitespace-pre-wrap break-all leading-relaxed">' + esc(m.content) + '</pre>'
            );
          }
        }
      }
      if (!messages.length) {
        msgsList.innerHTML = '<div class="text-center py-12 text-xs text-zinc-600">Empty session</div>';
      }
    }
  } catch (e) {
    msgsList.innerHTML = '<div class="text-center py-12 text-xs text-red-400">Failed to load: ' + esc(e.message) + '</div>';
  }
  loadSessions();
  if (sidebarOpen) toggleSidebar();
}

async function delSession(id) {
  if (!confirm('Delete this session?')) return;
  try {
    await fetch(API + '/api/sessions/' + id, { method: 'DELETE' });
    if (activeSessionId === id) {
      activeSessionId = null;
      document.getElementById('msgsList').innerHTML = '';
      document.getElementById('msgsList').classList.add('hidden');
      document.getElementById('emptyChat').classList.remove('hidden');
    }
    loadSessions();
  } catch (e) { alert('Delete failed: ' + e.message); }
}

function newChat() {
  activeSessionId = null;
  document.getElementById('msgsList').innerHTML = '';
  document.getElementById('msgsList').classList.add('hidden');
  document.getElementById('emptyChat').classList.remove('hidden');
  showTab('chat');
  loadSessions();
  document.getElementById('inp').focus();
  if (sidebarOpen) toggleSidebar();
}

/* ── Config ── */
async function loadCfg() {
  try {
    var r = await fetch(API + '/api/config');
    var c = await r.json();
    var prio = (c.provider && c.provider.priority ? c.provider.priority : ['local'])[0];
    document.getElementById('cProv').value = prio;
    var e = (c.provider && c.provider[prio]) ? c.provider[prio] : {};
    var maskedKey = e.api_key || '';
    document.getElementById('cKey').value = maskedKey;
    document.getElementById('cKey').placeholder = maskedKey ? 'Key saved (enter new to change)' : 'Enter API key';
    document.getElementById('cModel').value = e.model || '';
    document.getElementById('cUrl').value = e.base_url || '';
    document.getElementById('cMaxTok').value = e.max_tokens || 4096;
    document.getElementById('cIter').value = (c.agent && c.agent.max_iterations) || 50;
    document.getElementById('cCtx').value = (c.agent && c.agent.context_window) || 200000;
    var t = c.tools || {};
    document.getElementById('tShell').checked = t.shell !== false;
    document.getElementById('tFs').checked = t.filesystem !== false;
    document.getElementById('tSs').checked = t.screenshot !== false;
    document.getElementById('tTouch').checked = t.touch !== false;
    document.getElementById('tKey').checked = t.key_event !== false;
    document.getElementById('tText').checked = t.text_input !== false;
    document.getElementById('tUi').checked = t.ui_dump !== false;
    document.getElementById('tSms').checked = t.sms !== false;
    document.getElementById('tCall').checked = t.call !== false;
  } catch (e) { console.error('loadCfg', e); }
  loadSoul();
}

function provChanged() {
  var p = document.getElementById('cProv').value;
  var urls = { local: '', openrouter: 'https://openrouter.ai/api/v1', anthropic: 'https://api.anthropic.com' };
  document.getElementById('cUrl').value = urls[p] || '';
}

async function saveCfg() {
  var prov = document.getElementById('cProv').value;
  var keyVal = document.getElementById('cKey').value;
  var cfg = {
    agent: {
      max_iterations: parseInt(document.getElementById('cIter').value) || 50,
      context_window: parseInt(document.getElementById('cCtx').value) || 200000
    },
    provider: { priority: [prov] },
    tools: {
      shell: document.getElementById('tShell').checked,
      filesystem: document.getElementById('tFs').checked,
      screenshot: document.getElementById('tSs').checked,
      touch: document.getElementById('tTouch').checked,
      key_event: document.getElementById('tKey').checked,
      text_input: document.getElementById('tText').checked,
      ui_dump: document.getElementById('tUi').checked,
      sms: document.getElementById('tSms').checked,
      call: document.getElementById('tCall').checked
    }
  };
  cfg.provider[prov] = {
    api_key: keyVal || null,
    model: document.getElementById('cModel').value,
    base_url: document.getElementById('cUrl').value || null,
    max_tokens: parseInt(document.getElementById('cMaxTok').value) || 4096
  };
  try {
    var r = await fetch(API + '/api/config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(cfg)
    });
    if (r.ok) {
      var el = document.getElementById('cfgSaved');
      el.classList.remove('hidden');
      el.classList.add('inline-flex');
      setTimeout(function() { el.classList.add('hidden'); el.classList.remove('inline-flex'); }, 3000);
      checkStatus();
    } else {
      alert('Save failed: ' + r.status);
    }
  } catch (e) { alert('Save error: ' + e.message); }
}

/* ── Status ── */
async function checkStatus() {
  try {
    var r = await fetch(API + '/api/status');
    var s = await r.json();
    var dotEl = document.getElementById('statusDot');
    dotEl.innerHTML =
      '<span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>' +
      '<span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span>';
    document.getElementById('modelInfo').textContent = s.model || 'unknown';
    if (s.memory) {
      var memEl = document.getElementById('memInfo');
      memEl.textContent = 'RSS: ' + (s.memory.rss_mb || '?') + 'MB';
      memEl.classList.remove('hidden');
      memEl.classList.add('inline');
    }
  } catch (e) {
    var dotEl2 = document.getElementById('statusDot');
    dotEl2.innerHTML = '<span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-red-500"></span>';
    document.getElementById('modelInfo').textContent = 'Offline';
    document.getElementById('memInfo').classList.add('hidden');
  }
}

/* ── Utils ── */
function esc(s) {
  var d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}
function escAttr(s) {
  return String(s).replace(/&/g,'&amp;').replace(/'/g,'&#39;').replace(/"/g,'&quot;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

/* ── Device Profile ── */
var profileLoaded = false;
async function loadProfile() {
  if (profileLoaded) return;
  try {
    var r = await fetch(API+'/api/device/profile');
    var p = await r.json();
    profileLoaded = true;
    document.getElementById('dpModel').textContent = p.identity.manufacturer + ' ' + p.identity.model;
    document.getElementById('dpAndroid').textContent = 'Android ' + p.android.version + ' (API ' + p.android.api_level + ')';
    document.getElementById('dpArch').textContent = p.hardware.cpu_abi + ' (' + p.hardware.cpu_cores + ' cores)';
    document.getElementById('dpScreen').textContent = p.screen.width + 'x' + p.screen.height + ' (' + p.screen.density_name + ')';
    document.getElementById('dpRam').textContent = p.hardware.ram_total_mb + ' MB';
    document.getElementById('dpSE').textContent = p.android.selinux + (p.android.rooted ? ' (rooted)' : '');

    // Tools
    var toolsEl = document.getElementById('devTools');
    toolsEl.innerHTML = p.tools.map(function(t) {
      return '<div class="flex items-center gap-1.5 px-2.5 py-1.5 bg-emerald-900/20 border border-emerald-800/30 rounded-lg">' +
        '<span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>' +
        '<span class="text-[11px] text-emerald-300 font-medium">' + esc(t.name) + '</span>' +
        '<span class="text-[9px] text-zinc-600">' + esc(t.method) + '</span></div>';
    }).join('');

    // Hardware capabilities
    var hwEl = document.getElementById('devHw');
    var caps = [
      {name:'Touchscreen', ok:p.hardware.has_touchscreen},
      {name:'Framebuffer', ok:p.hardware.has_framebuffer},
      {name:'Modem', ok:p.hardware.has_modem},
      {name:'WiFi', ok:p.hardware.has_wifi},
      {name:'Camera', ok:p.hardware.has_camera},
    ];
    hwEl.innerHTML = caps.map(function(c) {
      var color = c.ok ? 'emerald' : 'zinc';
      return '<div class="flex items-center gap-1.5 px-2.5 py-1.5 bg-'+color+'-900/20 border border-'+color+'-800/30 rounded-lg">' +
        '<span class="w-1.5 h-1.5 rounded-full bg-'+color+'-'+(c.ok?'400':'600')+'"></span>' +
        '<span class="text-[11px] text-'+color+'-'+(c.ok?'300':'500')+'">' + c.name + '</span></div>';
    }).join('');

    if (p.hardware.input_devices.length) {
      hwEl.innerHTML += '<div class="w-full mt-2 text-[10px] text-zinc-600">Input: ' +
        p.hardware.input_devices.map(esc).join(', ') + '</div>';
    }
  } catch(e) { console.error('profile',e); }
}

/* ── Monitor ── */
async function refreshStats() {
  loadProfile();
  try {
    var r = await fetch(API+'/api/device/stats');
    var s = await r.json();
    document.getElementById('sCpu').textContent = s.cpu.usage_percent.toFixed(1)+'%';
    document.getElementById('sMem').textContent = s.memory.used_percent.toFixed(0)+'%';
    document.getElementById('sMemDetail').textContent = s.memory.used_mb+'MB / '+s.memory.total_mb+'MB';
    document.getElementById('sBat').textContent = s.battery.level >= 0 ? s.battery.level+'%' : 'N/A';
    document.getElementById('sBatDetail').textContent = s.battery.status+(s.battery.temperature > 0 ? ' '+s.battery.temperature+'C' : '');
    document.getElementById('sDisk').textContent = s.disk.data_used_percent.toFixed(0)+'%';
    document.getElementById('sDiskDetail').textContent = s.disk.data_free_mb+'MB free';
    document.getElementById('sNet').textContent = s.network.ip_address;
    document.getElementById('sUp').textContent = s.uptime;
    document.getElementById('sHrss').textContent = s.memory.peko_rss_mb.toFixed(1)+'MB';
    document.getElementById('sLoad').textContent = s.cpu.load_avg;

    var procsEl = document.getElementById('sProcs');
    procsEl.innerHTML = s.processes.map(function(p){
      return '<div class="flex justify-between"><span class="text-zinc-300 truncate mr-4">'+esc(p.name)+'</span><span>PID:'+p.pid+' RSS:'+p.rss_kb+'KB</span></div>';
    }).join('');
  } catch(e) { console.error('stats',e); }
}

var monitorInterval = null;
function startMonitorAutoRefresh() {
  if (monitorInterval) return;
  monitorInterval = setInterval(refreshStats, 3000);
}
function stopMonitorAutoRefresh() {
  if (monitorInterval) { clearInterval(monitorInterval); monitorInterval = null; }
}

var logES = null;
function toggleLogs() {
  var btn = document.getElementById('logToggle');
  if (logES) {
    logES.close(); logES = null;
    btn.textContent = 'Start'; btn.className = 'text-[10px] px-2 py-1 bg-emerald-900/40 border border-emerald-800/30 rounded text-emerald-400';
    return;
  }
  btn.textContent = 'Stop'; btn.className = 'text-[10px] px-2 py-1 bg-red-900/40 border border-red-800/30 rounded text-red-400';
  var box = document.getElementById('logBox'); box.textContent = '';
  logES = new EventSource(API+'/api/device/logs');
  logES.onmessage = function(e) {
    try {
      var d = JSON.parse(e.data);
      if (d.type === 'log') {
        box.textContent += d.line + '\n';
        if (box.childNodes.length > 500) box.textContent = box.textContent.split('\n').slice(-300).join('\n');
        box.scrollTop = box.scrollHeight;
      }
    } catch(e){}
  };
  logES.onerror = function() { toggleLogs(); };
}

/* ── Apps ── */
var allApps = [];
var appFilter = 'user';

async function loadApps() {
  var el = document.getElementById('appsList');
  el.innerHTML = '<p class="text-zinc-500 text-xs py-8 text-center">Loading apps...</p>';
  try {
    var r = await fetch(API+'/api/apps?filter='+appFilter);
    allApps = await r.json();
    filterApps();
  } catch(e) { console.error('apps',e); el.innerHTML = '<p class="text-red-400 text-xs">Failed to load</p>'; }
}

function setAppFilter(f) {
  appFilter = f;
  var on = 'px-2.5 py-1 rounded-md text-[10px] font-semibold bg-violet-600 text-white';
  var off = 'px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200';
  document.getElementById('afUser').className = f==='user' ? on : off;
  document.getElementById('afSystem').className = f==='system' ? on : off;
  document.getElementById('afAll').className = f==='all' ? on : off;
  loadApps();
}

function filterApps() {
  var q = document.getElementById('appSearch').value.toLowerCase();
  var filtered = allApps.filter(function(a){
    return a.package.toLowerCase().includes(q) || a.label.toLowerCase().includes(q);
  });
  document.getElementById('appsCount').textContent = filtered.length + ' of ' + allApps.length + ' apps';
  renderApps(filtered);
}

function renderApps(apps) {
  var el = document.getElementById('appsList');
  if (!apps.length) { el.innerHTML = '<p class="text-zinc-600 text-xs py-4 text-center">No apps found</p>'; return; }
  el.innerHTML = apps.map(function(a) {
    var icon = a.icon
      ? '<img src="'+escAttr(a.icon)+'" class="w-8 h-8 rounded-lg flex-shrink-0" onerror="this.style.display=\'none\'">'
      : '<div class="w-8 h-8 rounded-lg bg-zinc-800 border border-zinc-700/50 flex items-center justify-center flex-shrink-0"><span class="text-zinc-500 text-[10px] font-bold">'+esc((a.label||a.package)[0].toUpperCase())+'</span></div>';
    var badge = a.app_type === 'user'
      ? '<span class="px-1.5 py-0.5 bg-violet-900/30 text-violet-400 rounded text-[9px]">USER</span>'
      : '<span class="px-1.5 py-0.5 bg-zinc-800 text-zinc-500 rounded text-[9px]">SYS</span>';
    var status = a.enabled ? '' : '<span class="px-1.5 py-0.5 bg-red-900/30 text-red-400 rounded text-[9px]">OFF</span>';
    return '<div class="flex items-center gap-3 bg-zinc-900 border border-zinc-800/60 rounded-lg px-3 py-2.5 hover:border-zinc-700 transition-colors">' +
      icon +
      '<div class="flex-1 min-w-0">' +
        '<div class="flex items-center gap-2"><span class="text-sm text-zinc-200 font-medium truncate">'+esc(a.label||a.package)+'</span>'+badge+status+'</div>' +
        '<div class="text-[10px] text-zinc-500 font-mono truncate">'+esc(a.package)+(a.version ? ' v'+esc(a.version) : '')+'</div>' +
      '</div>' +
      '<div class="flex gap-1 flex-shrink-0">' +
      '<button onclick="appAct(\''+escAttr(a.package)+'\',\'launch\')" class="text-[10px] px-2 py-1 bg-emerald-900/30 hover:bg-emerald-900/50 text-emerald-400 rounded">Launch</button>' +
      '<button onclick="appAct(\''+escAttr(a.package)+'\',\'stop\')" class="text-[10px] px-2 py-1 bg-zinc-800 hover:bg-zinc-700 text-zinc-400 rounded">Stop</button>' +
      (a.app_type==='user' ? '<button onclick="appAct(\''+escAttr(a.package)+'\',\'uninstall\')" class="text-[10px] px-2 py-1 bg-red-900/30 hover:bg-red-900/50 text-red-400 rounded">Del</button>' : '') +
      '</div></div>';
  }).join('');
}

async function appAct(pkg, action) {
  if (action === 'uninstall' && !confirm('Uninstall '+pkg+'?')) return;
  try {
    var r = await fetch(API+'/api/apps/action', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({package:pkg,action:action})});
    var d = await r.json();
    alert(d.result || 'Done');
    if (action === 'uninstall') loadApps();
  } catch(e) { alert('Failed: '+e.message); }
}

/* ── Messages ── */
var msgES = null;
function startMsgStream() {
  var btn = document.getElementById('msgStreamBtn');
  if (msgES) {
    msgES.close(); msgES = null;
    btn.textContent = 'Disconnected'; return;
  }
  btn.textContent = 'Live';
  msgES = new EventSource(API+'/api/messages/stream');
  msgES.onmessage = function(e) {
    try {
      var d = JSON.parse(e.data);
      switch(d.type) {
        case 'sms_history': case 'sms_update':
          renderSms(d.messages); break;
        case 'notifications':
          renderNotifs(d.items); break;
        case 'sms_event': case 'notification_event':
          var ev = document.getElementById('msgEvents');
          ev.textContent += d.line + '\n';
          ev.scrollTop = ev.scrollHeight;
          break;
      }
    } catch(e){}
  };
  msgES.onerror = function() {
    btn.textContent = 'Reconnect';
    msgES = null;
  };
}

function renderSms(messages) {
  var el = document.getElementById('smsList');
  if (!messages || !messages.length) { el.innerHTML = '<p class="text-zinc-600 text-xs">No SMS messages</p>'; return; }
  el.innerHTML = messages.map(function(m) {
    return '<div class="bg-zinc-800/40 rounded-lg px-3 py-2 border border-zinc-800/60">' +
      '<div class="flex justify-between text-[10px] text-zinc-500 mb-1"><span>'+esc(m.from||'Unknown')+'</span><span>'+esc(m.date||'')+'</span></div>' +
      '<div class="text-xs text-zinc-300">'+esc(m.body||'')+'</div></div>';
  }).join('');
}

function renderNotifs(items) {
  var el = document.getElementById('notifList');
  if (!items || !items.length) { el.innerHTML = '<p class="text-zinc-600 text-xs">No notifications</p>'; return; }
  el.innerHTML = items.map(function(n) {
    return '<div class="bg-zinc-800/40 rounded-lg px-3 py-2 border border-zinc-800/60">' +
      '<span class="text-[10px] text-violet-400 font-mono">'+esc(n.package||'')+'</span>' +
      '<div class="text-xs text-zinc-300 mt-0.5">'+esc(n.text||'(no text)')+'</div></div>';
  }).join('');
}

/* ── Memory UI ── */
var allMemories = [];
var memFilter = 'all';

async function loadMemories() {
  try {
    var r = await fetch(API+'/api/memories');
    var d = await r.json();
    allMemories = d.memories || [];
    document.getElementById('memCount').textContent = d.count + ' memories';
    renderMemories(allMemories);
  } catch(e) { console.error('memories',e); }
}

function filterMem(cat) {
  memFilter = cat;
  var on = 'px-2.5 py-1 rounded-md text-[10px] font-semibold bg-violet-600 text-white';
  var off = 'px-2.5 py-1 rounded-md text-[10px] font-semibold text-zinc-400 hover:text-zinc-200';
  ['All','Fact','Pref','Proc','Obs','Skill'].forEach(function(n) {
    document.getElementById('mf'+n).className = off;
  });
  var btnMap = {all:'All',fact:'Fact',preference:'Pref',procedure:'Proc',observation:'Obs',skill:'Skill'};
  document.getElementById('mf'+(btnMap[cat]||'All')).className = on;

  var filtered = cat === 'all' ? allMemories : allMemories.filter(function(m){ return m.category === cat; });
  renderMemories(filtered);
}

function searchMemories() {
  var q = document.getElementById('memSearch').value.toLowerCase();
  var filtered = allMemories.filter(function(m) {
    return m.key.toLowerCase().includes(q) || m.content.toLowerCase().includes(q);
  });
  if (memFilter !== 'all') filtered = filtered.filter(function(m){ return m.category === memFilter; });
  renderMemories(filtered);
}

function renderMemories(mems) {
  var el = document.getElementById('memList');
  if (!mems.length) {
    el.innerHTML = '<div class="flex flex-col items-center py-12 text-zinc-600"><svg class="w-8 h-8 mb-2" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg><p class="text-xs">No memories yet. The agent will save important facts as it works.</p></div>';
    return;
  }
  el.innerHTML = mems.map(function(m) {
    var catColors = {fact:'blue',preference:'purple',procedure:'emerald',observation:'amber',skill:'cyan'};
    var c = catColors[m.category] || 'zinc';
    var impBar = '<div class="w-12 h-1 bg-zinc-800 rounded-full overflow-hidden"><div class="h-full bg-'+c+'-500 rounded-full" style="width:'+Math.round(m.importance*100)+'%"></div></div>';
    return '<div class="bg-zinc-900 border border-zinc-800/60 rounded-xl px-4 py-3 hover:border-zinc-700 transition-colors">' +
      '<div class="flex items-start justify-between gap-3">' +
        '<div class="flex-1 min-w-0">' +
          '<div class="flex items-center gap-2 mb-1">' +
            '<span class="text-sm font-medium text-zinc-200">'+esc(m.key)+'</span>' +
            '<span class="px-1.5 py-0.5 bg-'+c+'-900/30 text-'+c+'-400 rounded text-[9px] font-semibold uppercase">'+esc(m.category)+'</span>' +
            impBar +
          '</div>' +
          '<p class="text-xs text-zinc-400 leading-relaxed">'+esc(m.content)+'</p>' +
          '<div class="flex gap-3 mt-1.5 text-[10px] text-zinc-600">' +
            '<span>Accessed '+m.access_count+'x</span>' +
            '<span>'+esc((m.created_at||'').slice(0,10))+'</span>' +
          '</div>' +
        '</div>' +
        '<button onclick="delMemory(\''+escAttr(m.id)+'\')" class="text-zinc-600 hover:text-red-400 p-1 flex-shrink-0" title="Delete">' +
          '<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>' +
        '</button>' +
      '</div>' +
    '</div>';
  }).join('');
}

async function delMemory(id) {
  if (!confirm('Delete this memory?')) return;
  await fetch(API+'/api/memories/'+id, {method:'DELETE'});
  loadMemories();
}

/* ── Skills UI ── */
async function loadSkills() {
  try {
    var r = await fetch(API+'/api/skills');
    var d = await r.json();
    var skills = d.skills || [];
    document.getElementById('skillCount').textContent = d.count + ' skills';
    renderSkills(skills);
  } catch(e) { console.error('skills',e); }
}

function renderSkills(skills) {
  var el = document.getElementById('skillList');
  if (!skills.length) {
    el.innerHTML = '<div class="flex flex-col items-center py-12 text-zinc-600"><svg class="w-8 h-8 mb-2" fill="none" stroke="currentColor" stroke-width="1.5" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M4.26 10.147a60.436 60.436 0 00-.491 6.347A48.627 48.627 0 0112 20.904a48.627 48.627 0 018.232-4.41 60.46 60.46 0 00-.491-6.347m-15.482 0a50.57 50.57 0 00-2.658-.813A59.905 59.905 0 0112 3.493a59.902 59.902 0 0110.399 5.84c-.896.248-1.783.52-2.658.814m-15.482 0A50.697 50.697 0 0112 13.489a50.702 50.702 0 017.74-3.342"/></svg><p class="text-xs">No skills yet. The agent learns skills when it completes multi-step tasks.</p></div>';
    return;
  }
  el.innerHTML = skills.map(function(s) {
    var rate = s.success_rate || 0;
    var uses = s.success_count + s.fail_count;
    var rateColor = rate >= 80 ? 'emerald' : rate >= 50 ? 'amber' : 'red';
    return '<div class="bg-zinc-900 border border-zinc-800/60 rounded-xl overflow-hidden hover:border-zinc-700 transition-colors">' +
      '<div class="px-4 py-3">' +
        '<div class="flex items-start justify-between">' +
          '<div class="flex-1">' +
            '<div class="flex items-center gap-2 mb-1">' +
              '<span class="text-sm font-semibold text-zinc-200">'+esc(s.name)+'</span>' +
              '<span class="px-1.5 py-0.5 bg-zinc-800 text-zinc-500 rounded text-[9px]">'+esc(s.category)+'</span>' +
              (uses > 0 ? '<span class="px-1.5 py-0.5 bg-'+rateColor+'-900/30 text-'+rateColor+'-400 rounded text-[9px]">'+Math.round(rate)+'% success</span>' : '') +
            '</div>' +
            '<p class="text-xs text-zinc-400">'+esc(s.description)+'</p>' +
            (s.tags && s.tags.length ? '<div class="flex gap-1 mt-1.5">'+s.tags.map(function(t){return '<span class="px-1.5 py-0.5 bg-zinc-800 text-zinc-500 rounded text-[9px]">'+esc(t)+'</span>';}).join('')+'</div>' : '') +
            '<div class="text-[10px] text-zinc-600 mt-1">'+uses+' uses | updated '+(s.updated_at||'').slice(0,10)+'</div>' +
          '</div>' +
          '<div class="flex gap-1 flex-shrink-0">' +
            '<button onclick="toggleSkillSteps(this)" class="text-[10px] px-2 py-1 bg-zinc-800 hover:bg-zinc-700 text-zinc-400 rounded">Steps</button>' +
            '<button onclick="delSkill(\''+escAttr(s.name)+'\')" class="text-[10px] px-2 py-1 bg-red-900/30 hover:bg-red-900/50 text-red-400 rounded">Del</button>' +
          '</div>' +
        '</div>' +
      '</div>' +
      '<div class="hidden border-t border-zinc-800/40 bg-zinc-950/50 px-4 py-3">' +
        '<pre class="text-[11px] font-mono text-zinc-400 whitespace-pre-wrap leading-relaxed">'+esc(s.steps)+'</pre>' +
      '</div>' +
    '</div>';
  }).join('');
}

function toggleSkillSteps(btn) {
  var panel = btn.closest('.bg-zinc-900').querySelector('.hidden, .block');
  if (panel.classList.contains('hidden')) {
    panel.classList.remove('hidden');
    panel.classList.add('block');
    btn.textContent = 'Hide';
  } else {
    panel.classList.add('hidden');
    panel.classList.remove('block');
    btn.textContent = 'Steps';
  }
}

async function delSkill(name) {
  if (!confirm('Delete skill "'+name+'"?')) return;
  await fetch(API+'/api/skills/'+encodeURIComponent(name), {method:'DELETE'});
  loadSkills();
}

/* ── SOUL.md ── */
async function loadSoul() {
  try {
    var r = await fetch(API+'/api/soul');
    var d = await r.json();
    var el = document.getElementById('soulEditor');
    if (el) el.value = d.soul || '';
  } catch(e) {}
}

async function saveSoul() {
  var el = document.getElementById('soulEditor');
  if (!el) return;
  try {
    var r = await fetch(API+'/api/soul', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({soul:el.value})});
    if (r.ok) {
      var msg = document.getElementById('soulSaved');
      if (msg) { msg.style.display = 'inline'; setTimeout(function(){msg.style.display='none'},2000); }
    }
  } catch(e) { alert('Failed: '+e.message); }
}

/* ── Init ── */
checkStatus();
loadSessions();
setInterval(checkStatus, 8000);
document.getElementById('inp').focus();
</script>
</body>
</html>
"##;
