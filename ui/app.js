/* Aureate front-end — plain JavaScript, no build step, no npm.
   All logic lives in Rust; this file only draws and forwards events. */
(() => {
  "use strict";

  const T = window.__TAURI__;
  const invoke = T.core.invoke;
  const dlg = T.dialog || null;
  const evt = T.event;

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

  const statusLabels = { todo: "待办", doing: "进行中", done: "已完成", blocked: "受阻" };
  const kindAccent = {
    实验卡: "exp", 论文卡: "paper", 想法卡: "idea", 会议卡: "meeting", 周卡: "week",
  };
  const paperWidth = { a4: 210, a5: 148, letter: 215.9, legal: 215.9 };

  const S = {
    vault: null,
    entries: [],
    cards: [],
    doc: null,
    card: null,
    activePath: null,
    mode: "write",
    cardView: "form",
    dirty: false,
    saved: true,
    dark: false,
    zoom: 0,
    options: defaultOptions(),
    cardbook: { theme: "academic", page: "a6", header: "", footerPageNumbers: true, showSections: true, fontSize: 9 },
    collapsed: new Set(),
    paletteHits: [],
    paletteSel: 0,
    previewToken: 0,
    mathCache: new Map(),
    imgCache: new Map(),
  };

  function defaultOptions() {
    return {
      theme: "github", paper: "a4", landscape: false, margin: "normal",
      header: "", footerPageNumbers: true, toc: false, tocOwnPage: false,
      h1PageBreak: false, fontSize: 11, lineHeight: 0.78, firstLineIndent: false,
      date: "", title: "", mathAsText: false,
    };
  }

  // ------------------------------------------------------------------ utils

  const esc = (s) => String(s == null ? "" : s).replace(/[&<>"]/g, (c) => (
    { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]
  ));

  function debounce(fn, ms) {
    let t;
    return (...args) => { clearTimeout(t); t = setTimeout(() => fn(...args), ms); };
  }

  function toast(text, kind = "", actions = []) {
    const box = document.createElement("div");
    box.className = `toast ${kind}`;
    const row = document.createElement("div");
    row.className = "row";
    const span = document.createElement("span");
    span.textContent = text;
    row.appendChild(span);
    if (actions.length) {
      const act = document.createElement("span");
      act.className = "act";
      actions.forEach((a) => {
        const b = document.createElement("button");
        b.textContent = a.label;
        b.onclick = () => { try { a.run(); } finally { box.remove(); } };
        act.appendChild(b);
      });
      row.appendChild(act);
    }
    box.appendChild(row);
    $("#toasts").appendChild(box);
    const life = kind === "error" ? 8000 : 4600;
    setTimeout(() => box.remove(), life);
    return box;
  }

  function baseName(p) {
    return String(p || "").split(/[\\/]/).pop().replace(/\.md$/, "");
  }

  function today() {
    const d = new Date();
    const p = (n) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
  }

  function snippetHtml(s) {
    return esc(s || "").replace(/\u0001/g, "<mark>").replace(/\u0002/g, "</mark>");
  }

  // ---------------------------------------------------------------- prefs

  let prefsTimer = 0;
  async function loadPrefs() {
    try {
      const p = await invoke("load_prefs");
      if (!p) return;
      S.dark = !!p.dark;
      S.zoom = p.zoom || 0;
      if (p.options) S.options = Object.assign(defaultOptions(), p.options);
      if (p.cardbook) S.cardbook = Object.assign(S.cardbook, p.cardbook);
      if (typeof p.split === "number") S.split = p.split;
    } catch { /* ignore corrupt prefs */ }
  }

  // Preferences are written into the app package (data/settings.json), never
  // into the home folder.  Debounced because sliders fire constantly.
  function savePrefs() {
    clearTimeout(prefsTimer);
    prefsTimer = setTimeout(() => {
      invoke("save_prefs", {
        prefs: {
          dark: S.dark, options: S.options, cardbook: S.cardbook,
          zoom: S.zoom, split: S.split,
        },
      }).catch(() => {});
    }, 400);
  }

  // Recents are kept in memory for the session: the tool should leave no
  // trace of what you opened.
  let recents = [];
  function addRecent(doc) {
    recents = recents.filter((r) => r.path !== doc.path);
    recents.unshift({ path: doc.path, title: doc.title });
    recents = recents.slice(0, 8);
    renderRecent();
  }
  function renderRecent() {
    const host = $("#recent-list");
    const list = recents;
    if (!list.length) { host.innerHTML = ""; return; }
    host.innerHTML = `<h4>最近</h4>` + list.map((r) =>
      `<div class="row" data-open="${esc(r.path)}">${esc(r.title)}</div>`).join("");
  }

  // ---------------------------------------------------------------- theme

  function applyTheme() {
    document.body.classList.toggle("dark", S.dark);
    $("#btn-theme").textContent = S.dark ? "☀" : "◐";
  }

  function themeClass() {
    return S.options.theme || "github";
  }

  function applyPaper() {
    const w = paperWidth[S.options.paper] || 210;
    document.documentElement.style.setProperty("--paper-w", `${w}mm`);
  }

  function applyZoom() {
    const sheet = $("#preview-sheet");
    const scroll = $("#preview-scroll");
    if (!sheet) return;
    if (!S.zoom) {
      sheet.style.zoom = "";
      const avail = scroll.clientWidth - 52;
      const natural = sheet.offsetWidth || 1;
      const z = Math.min(1.25, Math.max(0.45, avail / natural));
      sheet.style.zoom = String(z);
      $("#zoom-level").textContent = "适应";
    } else {
      sheet.style.zoom = String(S.zoom);
      $("#zoom-level").textContent = `${Math.round(S.zoom * 100)}%`;
    }
  }

  // ---------------------------------------------------------------- vault

  async function boot() {
    await loadPrefs();
    applyTheme();
    applyPaper();
    bindChrome();
    bindEditor();
    bindPreview();
    bindKeys();
    bindDrop();

    // Always start from the picker: the app never reopens a folder on its own.
    showOnboard();

    evt.listen("vault-changed", debounce(async () => {
      if (!S.vault) return;
      await refreshAll();
    }, 700));

    evt.listen("export-progress", (e) => {
      const p = e.payload || {};
      if (p.error) {
        const line = document.createElement("div");
        line.className = "batch-line fail";
        line.textContent = `${baseName(p.input)}：${p.error}`;
        $("#status-info").textContent = "导出遇到错误";
        line.dataset.batch = "1";
        $("#toasts").appendChild(line);
        setTimeout(() => line.remove(), 8000);
      } else {
        $("#status-info").textContent = `已导出 ${baseName(p.output)} · ${p.pages} 页`;
      }
    });

    setInterval(() => { if (S.dirty) scheduleSave(); }, 15000);
  }

  function showOnboard() {
    $("#onboard").classList.remove("hidden");
  }

  async function openSingleFile() {
    if (!dlg) return;
    const picked = await dlg.open({
      multiple: false,
      title: "打开 Markdown 文件",
      filters: [{ name: "Markdown", extensions: ["md", "markdown"] }],
    });
    if (!picked) return;
    try {
      const doc = await invoke("open_file", { path: picked });
      await afterOpenFile(doc);
    } catch (e) {
      toast(`打开失败：${e}`, "error");
    }
  }

  async function afterOpenFile(doc) {
    const info = await invoke("vault_info");
    if (info) {
      S.vault = info;
      $("#vault-foot").textContent = info.root;
      $("#vault-foot").title = info.root;
    }
    $("#onboard").classList.add("hidden");
    recents = [];
    await refreshAll();
    openDocDto(doc);
    toast(`已打开 ${doc.title}`, "ok");
  }

  async function pickVault() {
    if (!dlg) return;
    const dir = await dlg.open({ directory: true, multiple: false, title: "选择 Aureate 仓库文件夹" });
    if (dir) await openVault(dir);
  }

  async function openVault(path) {
    const info = await invoke("open_vault", { path });
    S.vault = info;
    recents = [];
    $("#onboard").classList.add("hidden");
    $("#vault-foot").textContent = info.root;
    $("#vault-foot").title = info.root;
    await refreshAll();
    toast(`已打开 ${info.name} · ${info.docs} 个文档 / ${info.cards} 张卡`, "ok");
    if (info.indexMode === "memory") {
      toast("索引暂不可用（只读或无写入权限）：搜索与反链本次关闭，读写和导出不受影响", "warn");
    }
  }

  async function refreshAll() {
    await Promise.all([loadEntries(), loadCards()]);
    renderTree();
    renderCardList();
    if (S.mode === "cards") renderCardsBody();
  }

  async function loadEntries() {
    try { S.entries = await invoke("list_entries"); } catch { S.entries = []; }
  }

  async function loadCards() {
    try { S.cards = await invoke("cards"); } catch { S.cards = []; }
  }

  // ---------------------------------------------------------------- tree

  function renderTree() {
    const host = $("#file-tree");
    if (!S.vault) { host.innerHTML = ""; return; }
    const rows = [];
    const hiddenPrefixes = Array.from(S.collapsed);
    const isHidden = (path) => hiddenPrefixes.some((p) => path.startsWith(p + "/"));

    for (const e of S.entries) {
      if (isHidden(e.path)) continue;
      const isOpen = !S.collapsed.has(e.path);
      // One fixed-width twist column for every row: that is what keeps names
      // in a straight line no matter how deep the row sits.
      const twist = e.isDir ? (isOpen ? "▾" : "▸") : "";
      const active = S.activePath === e.path ? " active" : "";
      const actions = e.isDir
        ? `<span class="actions"><button data-act="new" data-path="${esc(e.path)}" title="在此新建文档">＋</button></span>`
        : `<span class="actions"><button data-act="rename" data-path="${esc(e.path)}" title="重命名">改</button><button data-act="delete" data-path="${esc(e.path)}" title="删除">删</button></span>`;
      rows.push(
        `<div class="row${active}" data-path="${esc(e.path)}" data-dir="${e.isDir}" style="--depth:${e.depth}">
           <span class="twist">${twist}</span>
           <span class="name">${esc(e.name.replace(/\.md$/, ""))}</span>${actions}
         </div>`);
    }
    host.innerHTML = rows.join("") || `<div class="empty" style="padding:24px 8px">空仓库。新建一个文档开始。</div>`;
  }

  // ---------------------------------------------------------------- editor

  let ta;
  function bindEditor() {
    ta = $("#editor-textarea");
    ta.addEventListener("input", () => {
      S.dirty = true;
      setSavedState(false);
      updateStatus();
      updateGutter();
      schedulePreview();
      scheduleSave();
    });
    ta.addEventListener("scroll", () => { $("#editor-gutter").scrollTop = ta.scrollTop; });
    ta.addEventListener("keydown", (e) => {
      if (e.key === "Tab") {
        e.preventDefault();
        const start = ta.selectionStart, end = ta.selectionEnd;
        ta.value = ta.value.slice(0, start) + "  " + ta.value.slice(end);
        ta.selectionStart = ta.selectionEnd = start + 2;
        ta.dispatchEvent(new Event("input"));
      }
    });
  }

  function setEditorValue(text) {
    ta.value = text;
    updateGutter();
  }

  function updateGutter() {
    const lines = ta.value.split("\n").length;
    const gutter = $("#editor-gutter");
    const nums = [];
    for (let i = 1; i <= lines; i++) nums.push(i);
    gutter.innerHTML = nums.join("<br>");
    gutter.scrollTop = ta.scrollTop;
  }

  function editorValue() { return ta ? ta.value : ""; }

  function setSavedState(saved) {
    S.saved = saved;
    const el = $("#status-save");
    el.textContent = saved ? "已保存" : "未保存";
    el.classList.toggle("dirty", !saved);
  }

  function updateStatus() {
    const text = editorValue();
    const cjk = (text.match(/[\u3400-\u9fff\u3040-\u30ff]/g) || []).length;
    const latin = (text.replace(/[\u3400-\u9fff\u3040-\u30ff]/g, " ").match(/[A-Za-z0-9_]+/g) || []).length;
    $("#status-words").textContent = `${cjk + latin} 字`;
    if (S.doc) $("#doc-title").textContent = S.doc.title;
  }

  const schedulePreview = debounce(() => renderPreview(), 180);
  const scheduleSave = debounce(() => { if (S.dirty) saveCurrent(true); }, 900);

  async function renderPreview() {
    if (!ta) return;
    const token = ++S.previewToken;
    const sheet = $("#preview-sheet");
    const content = editorValue();
    let html;
    try {
      html = await invoke("preview", { path: S.activePath, content, dark: S.dark });
    } catch (e) {
      sheet.innerHTML = `<div class="empty">预览失败：${esc(e)}</div>`;
      return;
    }
    if (token !== S.previewToken) return;
    sheet.className = `sheet ${themeClass()}`;
    sheet.innerHTML = html;
    await hydrate(sheet, token, null);
    applyZoom();
  }

  async function hydrate(sheet, token, card) {
    // Math: Typst renders each formula to SVG, so preview and PDF agree.
    const maths = $$(".math-inline, .math-block", sheet);
    await Promise.all(maths.map(async (el) => {
      const tex = el.dataset.tex || "";
      if (!tex) return;
      const display = el.classList.contains("math-block");
      const key = `${display ? "b" : "i"}|${S.options.fontSize}|${tex}`;
      let svg = S.mathCache.get(key);
      if (!svg) {
        try {
          svg = await invoke("preview_math", { tex, display, size: S.options.fontSize });
          S.mathCache.set(key, svg);
        } catch { return; }
      }
      if (token !== S.previewToken) return;
      el.innerHTML = `<span class="math-svg">${svg}</span>`;
    }));

    // Images: local paths are streamed as data URLs by Rust.
    const imgs = $$("img[data-asset]", sheet);
    await Promise.all(imgs.map(async (img) => {
      const asset = img.dataset.asset;
      let url = S.imgCache.get(asset);
      if (!url) {
        try {
          url = await invoke("read_image", { path: asset });
          S.imgCache.set(asset, url);
        } catch {
          img.replaceWith(Object.assign(document.createElement("span"), {
            className: "missing-image", textContent: `图片缺失：${asset}`,
          }));
          return;
        }
      }
      if (token !== S.previewToken) return;
      img.src = url;
    }));

    // Code actions
    $$(".code-copy", sheet).forEach((btn, idx) => {
      const block = btn.closest(".code-block");
      btn.onclick = async () => {
        const code = block?.querySelector("code")?.textContent || "";
        const inCommands = btn.closest('[data-section="关键命令"]');
        if (inCommands && card) {
          try {
            const p = await invoke("save_script", { cardId: card.id, index: idx, code });
            toast(`已存为脚本 ${p}`, "ok");
          } catch (e) { toast(`保存失败：${e}`, "error"); }
        } else {
          await copyText(code);
          btn.textContent = "已复制";
          setTimeout(() => (btn.textContent = "复制"), 1200);
        }
      };
      if (card && btn.closest('[data-section="关键命令"]')) btn.textContent = "存为脚本";
    });
    $$("a.wikilink", sheet).forEach((a) => {
      a.onclick = async (e) => {
        e.preventDefault();
        try {
          const doc = await invoke("open_card", { id: a.dataset.card });
          openDocDto(doc);
        } catch { toast(`找不到卡片 ${a.dataset.card}`, "error"); }
      };
    });
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      const t = document.createElement("textarea");
      t.value = text;
      document.body.appendChild(t);
      t.select();
      document.execCommand("copy");
      t.remove();
      return true;
    }
  }

  function bindPreview() {
    $("#zoom-in").onclick = () => {
      S.zoom = Math.min(1.6, (S.zoom || 0.9) + 0.1);
      applyZoom(); savePrefs();
    };
    $("#zoom-out").onclick = () => {
      S.zoom = Math.max(0.4, (S.zoom || 0.9) - 0.1);
      applyZoom(); savePrefs();
    };
    window.addEventListener("resize", debounce(applyZoom, 120));
  }

  // ---------------------------------------------------------------- docs

  async function openDoc(path) {
    try {
      const doc = await invoke("read_doc", { path });
      openDocDto(doc);
    } catch (e) {
      toast(`打开失败：${e}`, "error");
    }
  }

  function openDocDto(doc) {
    S.doc = doc;
    S.card = doc.card;
    S.activePath = doc.path;
    S.dirty = false;
    S.saved = true;
    setEditorValue(doc.content);
    updateStatus();
    setSavedState(true);
    addRecent(doc);
    renderTree();
    if (doc.card) {
      setMode("cards");
      setCardView("form");
      renderCardForm(doc);
      renderCardList();
    } else {
      setMode("write");
      renderPreview();
    }
    $("#sidebar").classList.remove("open");
  }

  async function saveCurrent(silent = false) {
    if (!S.doc) return;
    if (S.mode === "cards" && S.card && $("#card-form")) {
      return saveCardForm(silent);
    }
    try {
      const res = await invoke("save_doc", { path: S.doc.path, content: editorValue() });
      const wasCard = !!res.card;
      S.doc = res;
      S.card = res.card;
      S.dirty = false;
      setSavedState(true);
      updateStatus();
      if (wasCard) await refreshAll();
      if (!silent) toast("已保存", "ok");
    } catch (e) {
      toast(`保存失败：${e}`, "error");
    }
  }

  // ---------------------------------------------------------------- cards

  function filteredCards() {
    const kind = $("#card-filter-kind").value;
    return S.cards.filter((c) => !kind || c.kind === kind);
  }

  function renderCardList() {
    const host = $("#card-list");
    const cards = filteredCards();
    if (!cards.length) {
      host.innerHTML = `<div class="empty" style="padding:24px 8px">还没有卡片。</div>`;
      return;
    }
    host.innerHTML = cards.map((c) => `
      <div class="card-item${S.activePath === c.path ? " active" : ""}" data-card-path="${esc(c.path)}">
        <div class="t">${esc(c.title)}</div>
        <div class="m">
          <span class="badge ${kindAccent[c.kind] || "other"}">${esc(c.kind)}</span>
          <span>${esc(c.date)}</span>
          <span class="badge ${esc(c.status)}">${statusLabels[c.status] || c.status}</span>
        </div>
      </div>`).join("");
  }

  function setMode(mode) {
    S.mode = mode;
    $("#mode-write").classList.toggle("active", mode === "write");
    $("#mode-cards").classList.toggle("active", mode === "cards");
    $("#write-view").classList.toggle("hidden", mode !== "write");
    $("#cards-view").classList.toggle("hidden", mode !== "cards");
    if (mode === "write") {
      setTimeout(() => { renderPreview(); applyZoom(); }, 0);
    }
  }

  function setCardView(view) {
    S.cardView = view;
    $$("[data-cardview]").forEach((b) => b.classList.toggle("active", b.dataset.cardview === view));
    renderCardsBody();
  }

  function renderCardsBody() {
    if (S.cardView === "form") {
      if (S.card && S.doc) renderCardForm(S.doc);
      else {
        $("#cards-body").innerHTML = `<div class="empty"><div class="big">◈</div>从左侧选一张卡，或新建一张。</div>`;
      }
    } else if (S.cardView === "timeline") renderTimeline();
    else if (S.cardView === "board") renderBoard();
    else renderLedger();
  }

  function renderCardForm(doc) {
    const c = doc.card;
    if (!c) return;
    const sections = c.sections.map((s) => `
      <div class="card-section" data-section="${esc(s.title)}">
        <div class="sec-head">${esc(s.title)}</div>
        <textarea spellcheck="false">${esc(s.markdown)}</textarea>
      </div>`).join("");

    $("#cards-body").innerHTML = `
      <div class="card-form" id="card-form">
        <div class="left">
          <div class="card-meta">
            <label class="wide">标题<input id="card-title" type="text" value="${esc(c.title)}" /></label>
            <label>类型<select id="card-kind">
              ${["实验卡", "论文卡", "想法卡", "会议卡", "周卡"].map((k) =>
                `<option ${k === c.kind ? "selected" : ""}>${k}</option>`).join("")}
            </select></label>
            <label>日期<input id="card-date" type="date" value="${esc(c.date)}" /></label>
            <label>项目<input id="card-project" type="text" value="${esc(c.project || "")}" placeholder="课题一" /></label>
            <label>状态<select id="card-status">
              ${["todo", "doing", "done", "blocked"].map((s) =>
                `<option value="${s}" ${s === c.status ? "selected" : ""}>${statusLabels[s]}</option>`).join("")}
            </select></label>
            <label class="wide">标签<input id="card-tags" type="text" value="${esc(c.tags.join(", "))}" placeholder="用逗号分隔" /></label>
          </div>
          <div class="sections" id="card-sections">${sections}</div>
          <div class="card-backlinks" id="card-backlinks"></div>
          <div class="row-end" style="display:flex;gap:8px">
            <button class="tool-btn" id="card-export">导出此卡</button>
            <button class="tool-btn" id="card-delete">删除</button>
            <span class="flex-1"></span>
            <button class="tool-btn primary" id="card-save">保存卡片</button>
          </div>
        </div>
        <div class="right">
          <div class="sheet ${themeClass()}" id="card-preview"></div>
        </div>
      </div>`;

    const changed = () => {
      S.dirty = true;
      setSavedState(false);
      scheduleCardPreview();
      scheduleSave();
    };
    $$("#card-form input, #card-form select, #card-form textarea").forEach((el) => {
      el.addEventListener("input", changed);
      el.addEventListener("change", changed);
    });

    $("#card-save").onclick = () => saveCardForm(false);
    $("#card-export").onclick = () => exportSingleCard(c);
    $("#card-delete").onclick = () => confirmModal("删除这张卡片？文件会被移到仓库的回收站。", async () => {
      await invoke("delete_entry", { path: c.path });
      await refreshAll();
      S.doc = null; S.card = null; S.activePath = null;
      renderCardsBody();
      toast("已移入回收站", "ok");
    });

    scheduleCardPreview(true);
    loadBacklinks(c.id);
  }

  function cardFormValue() {
    const title = $("#card-title")?.value.trim() || "未命名卡片";
    const kind = $("#card-kind")?.value || "实验卡";
    const date = $("#card-date")?.value || today();
    const project = $("#card-project")?.value.trim() || null;
    const status = $("#card-status")?.value || "todo";
    const tags = ($("#card-tags")?.value || "").split(",").map((t) => t.trim()).filter(Boolean);
    const sections = $$("#card-sections .card-section").map((el) => ({
      title: el.dataset.section,
      markdown: el.querySelector("textarea").value,
    }));
    return { meta: { title, kind, date, project, status, tags }, sections };
  }

  function composeCardMarkdown() {
    const v = cardFormValue();
    let md = `# ${v.meta.title}\n\n`;
    for (const s of v.sections) {
      if (s.title === "正文") md += s.markdown + "\n\n";
      else md += `## ${s.title}\n\n${s.markdown}\n\n`;
    }
    return md;
  }

  const scheduleCardPreview = debounce(() => renderCardPreview(), 220);

  async function renderCardPreview() {
    if (!$("#card-preview")) return;
    const token = ++S.previewToken;
    const sheet = $("#card-preview");
    const md = composeCardMarkdown();
    try {
      const html = await invoke("preview", { path: S.doc?.path || null, content: md, dark: S.dark });
      if (token !== S.previewToken) return;
      sheet.innerHTML = html;
      await hydrate(sheet, token, S.card);
    } catch (e) {
      sheet.innerHTML = `<div class="empty">${esc(e)}</div>`;
    }
  }

  async function saveCardForm(silent) {
    if (!S.doc || !S.card) return;
    const v = cardFormValue();
    try {
      const doc = await invoke("save_card", { path: S.doc.path, meta: v.meta, sections: v.sections });
      S.doc = doc;
      S.card = doc.card;
      S.dirty = false;
      setSavedState(true);
      await loadCards();
      renderCardList();
      if (!silent) toast("卡片已保存", "ok");
    } catch (e) {
      toast(`保存失败：${e}`, "error");
    }
  }

  async function loadBacklinks(id) {
    const host = $("#card-backlinks");
    if (!host) return;
    try {
      const links = await invoke("backlinks", { id });
      host.innerHTML = links.length
        ? "被引用：" + links.map((l) =>
            `<a data-bl="${esc(l.path)}">${esc(l.title)}</a>`).join("")
        : "还没有卡片引用这张卡。在正文里写 [[id]] 即可。";
      $$("#card-backlinks a").forEach((a) => {
        a.onclick = () => openDoc(a.dataset.bl);
      });
    } catch { host.textContent = ""; }
  }

  // ---------------------------------------------------------------- views

  function renderTimeline() {
    const cards = filteredCards();
    if (!cards.length) {
      $("#cards-body").innerHTML = `<div class="empty"><div class="big">◷</div>这个时间段还没有卡片。</div>`;
      return;
    }
    const groups = new Map();
    for (const c of cards) {
      if (!groups.has(c.date)) groups.set(c.date, []);
      groups.get(c.date).push(c);
    }
    const days = Array.from(groups.keys()).sort().reverse();
    $("#cards-body").innerHTML = `<div class="timeline">` + days.map((d) => {
      const list = groups.get(d);
      return `<div class="day-head"><span class="d">${esc(d)}</span><span>${list.length} 张</span></div>
        <div class="items">${list.map((c) => `
          <div class="card-item" data-card-path="${esc(c.path)}">
            <div class="t">${esc(c.title)}</div>
            <div class="m"><span class="badge ${kindAccent[c.kind] || "other"}">${esc(c.kind)}</span>
              <span class="badge ${esc(c.status)}">${statusLabels[c.status] || c.status}</span>
              ${c.project ? `<span>${esc(c.project)}</span>` : ""}</div>
          </div>`).join("")}</div>`;
    }).join("") + `</div>`;
    bindCardOpeners();
  }

  function renderBoard() {
    const cols = [
      ["todo", "待办"], ["doing", "进行中"], ["done", "已完成"], ["blocked", "受阻"],
    ];
    const cards = filteredCards();
    $("#cards-body").innerHTML = `<div class="board">` + cols.map(([key, label]) => {
      const list = cards.filter((c) => c.status === key);
      return `<div class="col" data-status="${key}">
        <h3>${label}<span class="count">${list.length}</span></h3>
        ${list.map((c) => `
          <div class="card-chip" draggable="true" data-id="${esc(c.id)}" data-card-path="${esc(c.path)}">
            <div>${esc(c.title)}</div>
            <div class="meta"><span class="badge ${kindAccent[c.kind] || "other"}">${esc(c.kind)}</span><span>${esc(c.date)}</span></div>
          </div>`).join("")}
      </div>`;
    }).join("") + `</div>`;

    bindCardOpeners();
    let dragged = null;
    $$(".card-chip").forEach((chip) => {
      chip.addEventListener("dragstart", () => { dragged = chip.dataset.id; });
      chip.addEventListener("dblclick", () => openDoc(chip.dataset.cardPath));
    });
    $$(".board .col").forEach((col) => {
      col.addEventListener("dragover", (e) => { e.preventDefault(); col.classList.add("drag-over"); });
      col.addEventListener("dragleave", () => col.classList.remove("drag-over"));
      col.addEventListener("drop", async (e) => {
        e.preventDefault();
        col.classList.remove("drag-over");
        if (!dragged) return;
        try {
          await invoke("update_card_status", { id: dragged, status: col.dataset.status });
          await loadCards();
          renderBoard();
        } catch (err) { toast(String(err), "error"); }
        dragged = null;
      });
    });
  }

  function bindCardOpeners() {
    $$("[data-card-path]").forEach((el) => {
      el.addEventListener("click", (e) => {
        if (e.target.closest(".card-chip")) return;
        openDoc(el.dataset.cardPath);
      });
    });
  }

  async function renderLedger() {
    try {
      const ledger = await invoke("ledger");
      const d = ledger.data;
      const maxKind = Math.max(1, ...d.byKind.map((x) => x[1]));
      const weekCards = S.cards.filter((c) => c.date >= ledger.weekStart && c.date <= ledger.weekEnd);
      $("#cards-body").innerHTML = `<div class="ledger">
        <div class="grid">
          <div class="stat"><div class="num">${d.total}</div><div class="cap">本周卡片</div></div>
          <div class="stat"><div class="num">${d.done}</div><div class="cap">已完成</div></div>
          <div class="stat"><div class="num">${d.doing}</div><div class="cap">进行中</div></div>
          <div class="stat"><div class="num">${d.activeDays}</div><div class="cap">活跃天数</div></div>
        </div>
        <div class="batch-line">${esc(ledger.weekStart)} → ${esc(ledger.weekEnd)}</div>
        <h3>按类型</h3>
        ${d.byKind.map(([k, n]) => `<div class="bar-row"><span class="label">${esc(k)}</span>
          <span class="bar" style="width:${(n / maxKind) * 60}%"></span><span>${n}</span></div>`).join("") || `<div class="batch-line">本周还没有卡片。</div>`}
        <h3>按项目</h3>
        ${d.byProject.map(([k, n]) => `<div class="bar-row"><span class="label">${esc(k)}</span>
          <span class="bar" style="width:${(n / Math.max(1, d.byProject[0][1])) * 60}%"></span><span>${n}</span></div>`).join("") || `<div class="batch-line">未分项目。</div>`}
        <h3>本周卡片</h3>
        ${weekCards.map((c) => `<div class="card-item" data-card-path="${esc(c.path)}">
          <div class="t">${esc(c.title)}</div>
          <div class="m"><span>${esc(c.date)}</span><span class="badge ${esc(c.status)}">${statusLabels[c.status]}</span></div>
        </div>`).join("") || `<div class="batch-line">空。</div>`}
      </div>`;
      bindCardOpeners();
    } catch (e) {
      $("#cards-body").innerHTML = `<div class="empty">${esc(e)}</div>`;
    }
  }

  // ---------------------------------------------------------------- create

  function modal(html) {
    const host = $("#modal");
    $("#modal-box").innerHTML = html;
    host.classList.remove("hidden");
    return $("#modal-box");
  }
  function closeModal() { $("#modal").classList.add("hidden"); }

  function confirmModal(text, run) {
    const box = modal(`<h3>确认</h3><p style="font-size:13.5px;color:var(--text-dim);line-height:1.7">${esc(text)}</p>
      <div class="row-end"><button class="tool-btn" id="m-cancel">取消</button>
      <button class="tool-btn primary" id="m-ok">确定</button></div>`);
    box.querySelector("#m-cancel").onclick = closeModal;
    box.querySelector("#m-ok").onclick = async () => { closeModal(); await run(); };
  }

  function newDocModal(dir) {
    const box = modal(`<h3>新建文档</h3>
      <input id="m-name" placeholder="文件名，例如 论文笔记" />
      <div class="row-end"><button class="tool-btn" id="m-cancel">取消</button>
      <button class="tool-btn primary" id="m-ok">创建</button></div>`);
    const input = box.querySelector("#m-name");
    input.focus();
    const go = async () => {
      const name = input.value.trim() || "未命名文档";
      closeModal();
      try {
        const doc = await invoke("create_doc", { dir: dir || "docs", name });
        await refreshAll();
        openDocDto(doc);
        ta.focus();
      } catch (e) { toast(String(e), "error"); }
    };
    box.querySelector("#m-ok").onclick = go;
    box.querySelector("#m-cancel").onclick = closeModal;
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") go(); });
  }

  function newCardModal() {
    const kinds = [
      ["实验卡", "问题 / 设计 / 命令 / 结果"],
      ["论文卡", "文献 / 方法 / 可复用点"],
      ["想法卡", "一句话 / 背景 / 可能性"],
      ["会议卡", "议题 / 结论 / 待办"],
      ["周卡", "本周完成 / 下周计划"],
    ];
    const box = modal(`<h3>新建卡片</h3>
      <div class="kind-grid">${kinds.map(([k, hint]) =>
        `<button data-kind="${k}"><b>${k}</b><div style="font-size:11px;color:var(--text-dim);margin-top:3px">${hint}</div></button>`).join("")}</div>
      <input id="m-title" placeholder="标题（可留空）" />
      <div class="row-end"><button class="tool-btn" id="m-cancel">取消</button></div>`);
    let kind = "实验卡";
    $$("button[data-kind]", box).forEach((b) => {
      b.onclick = async () => {
        kind = b.dataset.kind;
        const title = box.querySelector("#m-title").value.trim() || null;
        closeModal();
        try {
          const doc = await invoke("create_card", { kind, title, project: null, body: null });
          await refreshAll();
          openDocDto(doc);
        } catch (e) { toast(String(e), "error"); }
      };
    });
    box.querySelector("#m-cancel").onclick = closeModal;
    box.querySelector("#m-title").focus();
  }

  // ---------------------------------------------------------------- export

  function suggestName() {
    const title = (S.card && S.mode === "cards" ? S.card.title : S.doc?.title) || "未命名文档";
    return `${title.replace(/[\\/:*?"<>|]/g, "_")}.pdf`;
  }

  async function askSavePath(name, ext) {
    if (!dlg) return null;
    try {
      return await dlg.save({
        defaultPath: name,
        filters: [{ name: ext.toUpperCase(), extensions: [ext.split(".").pop()] }],
      });
    } catch { return null; }
  }

  async function exportCurrent() {
    if (!S.doc && S.mode === "write") {
      toast("先打开或新建一个文档", "error");
      return;
    }
    if (S.mode === "cards" && S.card && $("#card-form")) {
      await saveCardForm(true);
    }
    const content = S.mode === "cards" && $("#card-form") ? composeCardMarkdown() : editorValue();
    let out = await askSavePath(suggestName(), "pdf");
    if (!out && S.vault) out = `${S.vault.root}/exports/${suggestName()}`;
    if (!out) return;
    const tip = toast("正在编译 PDF…");
    try {
      const res = await invoke("export_pdf", {
        path: S.card && S.mode === "cards" ? S.card.path : S.doc?.path || null,
        content, out, options: S.options,
      });
      tip.remove();
      toast(`已导出 ${res.pages} 页 · ${(res.bytes / 1024).toFixed(0)} KB`, "ok", [
        { label: "打开", run: () => invoke("open_external", { path: res.output }) },
        { label: "显示", run: () => invoke("reveal", { path: res.output }) },
      ]);
      if (res.warnings && res.warnings.length) {
        toast(res.warnings[0], "error");
      }
      $("#status-info").textContent = `导出 ${res.pages} 页`;
    } catch (e) {
      tip.remove();
      toast(`导出失败：${e}`, "error");
    }
  }

  async function exportSingleCard(c) {
    await saveCardForm(true);
    let out = await askSavePath(`${c.title}.pdf`, "pdf");
    if (!out && S.vault) out = `${S.vault.root}/exports/${c.title}.pdf`;
    if (!out) return;
    try {
      const res = await invoke("export_cardbook", { paths: [c.path], out, options: S.cardbook });
      toast(`卡片已导出 · ${res.pages} 页`, "ok", [
        { label: "打开", run: () => invoke("open_external", { path: res.output }) },
      ]);
    } catch (e) { toast(`导出失败：${e}`, "error"); }
  }

  async function exportCardbook() {
    const cards = filteredCards();
    if (!cards.length) { toast("当前筛选下没有卡片", "error"); return; }
    let out = await askSavePath(`卡册-${today()}.pdf`, "pdf");
    if (!out && S.vault) out = `${S.vault.root}/exports/卡册-${today()}.pdf`;
    if (!out) return;
    const tip = toast(`正在拼装 ${cards.length} 张卡…`);
    try {
      const res = await invoke("export_cardbook", {
        paths: cards.map((c) => c.path), out, options: S.cardbook,
      });
      tip.remove();
      toast(`卡册已导出 · ${res.pages} 页`, "ok", [
        { label: "打开", run: () => invoke("open_external", { path: res.output }) },
      ]);
    } catch (e) { tip.remove(); toast(`导出失败：${e}`, "error"); }
  }

  async function batchExport() {
    if (!dlg) return;
    const dir = await dlg.open({ directory: true, multiple: false, title: "选择要批量导出的文件夹" });
    if (!dir) return;
    const box = modal(`<h3>批量导出</h3>
      <p style="font-size:13px;color:var(--text-dim);line-height:1.7">递归查找该文件夹下的 Markdown，逐个编译为同名 PDF。失败不会中断整批。</p>
      <div class="row-end">
        <button class="tool-btn" id="b-same">输出到原目录</button>
        <button class="tool-btn primary" id="b-pick">选择输出目录</button>
      </div>`);
    const run = async (outDir) => {
      closeModal();
      const tip = toast("准备批量导出…");
      try {
        const items = await invoke("export_folder", { dir, outDir, options: S.options });
        tip.remove();
        const fail = items.filter((i) => i.error);
        toast(`批量完成：成功 ${items.length - fail.length} / ${items.length}`, fail.length ? "error" : "ok",
          fail.length ? [{ label: "看失败项", run: () => showBatchFailures(fail) }] : []);
      } catch (e) { tip.remove(); toast(`批量导出失败：${e}`, "error"); }
    };
    box.querySelector("#b-same").onclick = () => run(null);
    box.querySelector("#b-pick").onclick = async () => {
      const outDir = await dlg.open({ directory: true, multiple: false, title: "选择输出目录" });
      if (outDir) await run(outDir);
    };
  }

  function showBatchFailures(items) {
    modal(`<h3>失败清单</h3><div style="max-height:300px;overflow:auto;font-size:12.5px">
      ${items.map((i) => `<div class="batch-line fail">${esc(baseName(i.input))}：${esc(i.error)}</div>`).join("")}
      </div><div class="row-end"><button class="tool-btn primary" id="m-ok">知道了</button></div>`)
      .querySelector("#m-ok").onclick = closeModal;
  }

  // ---------------------------------------------------------------- settings

  function renderSettings() {
    const o = S.options;
    const fontInfo = $("#font-status");
    $("#settings-body").innerHTML = `
      <label class="field">主题<select id="s-theme">
        ${[["github", "GitHub"], ["academic", "学术论文"], ["business", "商务报告"], ["minimal", "极简"]]
          .map(([v, l]) => `<option value="${v}" ${o.theme === v ? "selected" : ""}>${l}</option>`).join("")}
      </select></label>
      <div class="field-row">
        <label class="field" style="flex:1">纸张<select id="s-paper">
          ${[["a4", "A4"], ["a5", "A5"], ["letter", "Letter"], ["legal", "Legal"]]
            .map(([v, l]) => `<option value="${v}" ${o.paper === v ? "selected" : ""}>${l}</option>`).join("")}
        </select></label>
        <label class="field" style="flex:1">页边距<select id="s-margin">
          ${[["narrow", "窄"], ["normal", "标准"], ["wide", "宽"]]
            .map(([v, l]) => `<option value="${v}" ${o.margin === v ? "selected" : ""}>${l}</option>`).join("")}
        </select></label>
      </div>
      <div class="field-row">
        <label class="field" style="flex:1">字号 (pt)<input id="s-size" type="number" min="8" max="16" step="0.5" value="${o.fontSize}" /></label>
        <label class="field" style="flex:1">行距<select id="s-leading">
          ${[0.7, 0.78, 0.85, 0.95, 1.05].map((v) =>
            `<option value="${v}" ${Math.abs(o.lineHeight - v) < 0.01 ? "selected" : ""}>${v.toFixed(2)}</option>`).join("")}
        </select></label>
      </div>
      <label class="field">页眉模板（支持 {title} 和 {date}）<input id="s-header" type="text" value="${esc(o.header)}" placeholder="留空则不显示页眉" /></label>
      <label class="field">日期（用于 {date}）<input id="s-date" type="date" value="${esc(o.date)}" /></label>
      <div class="check-row"><input type="checkbox" id="s-footer" ${o.footerPageNumbers ? "checked" : ""}/>页脚显示「第 X 页 / 共 Y 页」</div>
      <div class="check-row"><input type="checkbox" id="s-landscape" ${o.landscape ? "checked" : ""}/>横向纸张</div>
      <div class="check-row"><input type="checkbox" id="s-toc" ${o.toc ? "checked" : ""}/>生成目录（H1–H3）</div>
      <div class="check-row"><input type="checkbox" id="s-tocpage" ${o.tocOwnPage ? "checked" : ""}/>目录单独占页</div>
      <div class="check-row"><input type="checkbox" id="s-h1" ${o.h1PageBreak ? "checked" : ""}/>一级标题另起一页</div>
      <div class="check-row"><input type="checkbox" id="s-indent" ${o.firstLineIndent ? "checked" : ""}/>首行缩进两字</div>
      <div class="check-row"><input type="checkbox" id="s-mathtext" ${o.mathAsText ? "checked" : ""}/>公式无法转换时显示源码</div>
      <div class="field" id="font-status">字体检查中…</div>`;
    void fontInfo;

    const apply = () => {
      o.theme = $("#s-theme").value;
      o.paper = $("#s-paper").value;
      o.margin = $("#s-margin").value;
      o.fontSize = parseFloat($("#s-size").value) || 11;
      o.lineHeight = parseFloat($("#s-leading").value) || 0.78;
      o.header = $("#s-header").value;
      o.date = $("#s-date").value;
      o.footerPageNumbers = $("#s-footer").checked;
      o.landscape = $("#s-landscape").checked;
      o.toc = $("#s-toc").checked;
      o.tocOwnPage = $("#s-tocpage").checked;
      o.h1PageBreak = $("#s-h1").checked;
      o.firstLineIndent = $("#s-indent").checked;
      o.mathAsText = $("#s-mathtext").checked;
      savePrefs();
      applyPaper();
      S.mathCache.clear();
      if (S.mode === "write") schedulePreview();
      else scheduleCardPreview();
      applyZoom();
    };
    $$("#settings-body input, #settings-body select").forEach((el) => {
      el.addEventListener("change", apply);
      el.addEventListener("input", apply);
    });

    invoke("font_report").then((f) => {
      const el = $("#font-status");
      if (!el) return;
      el.innerHTML = `<div class="batch-line">正文字体：${esc(f.serif)} · 标题：${esc(f.sans)}</div>
        <div class="batch-line">${f.hasCjk ? "中文嵌入正常" : "缺少中文字体，PDF 可能缺字"}</div>
        ${f.warnings.map((w) => `<div class="batch-line fail">${esc(w)}</div>`).join("")}`;
    }).catch(() => {});
  }

  // ---------------------------------------------------------------- palette

  let paletteTimer;
  function openPalette() {
    $("#palette").classList.remove("hidden");
    const input = $("#palette-input");
    input.value = "";
    input.focus();
    runSearch("");
  }

  function closePalette() {
    $("#palette").classList.add("hidden");
  }

  async function runSearch(q) {
    try {
      const hits = await invoke("search", { query: q, limit: 30 });
      S.paletteHits = hits;
      S.paletteSel = 0;
      renderPalette();
    } catch { S.paletteHits = []; renderPalette(); }
  }

  function renderPalette() {
    const host = $("#palette-results");
    if (!S.paletteHits.length) {
      host.innerHTML = `<div class="batch-line" style="padding:14px">没有结果</div>`;
      return;
    }
    host.innerHTML = S.paletteHits.map((h, i) => `
      <div class="hit${i === S.paletteSel ? " sel" : ""}" data-i="${i}">
        <span class="badge ${kindAccent[h.kind] || "other"}">${esc(h.kind || "文档")}</span>
        <span>${esc(h.title)}</span>
        <span class="snippet">${snippetHtml(h.snippet)}</span>
        <span class="path">${esc(h.path)}</span>
      </div>`).join("");
    $$("#palette-results .hit").forEach((el) => {
      el.onclick = () => pickPalette(Number(el.dataset.i));
      el.onmouseenter = () => { S.paletteSel = Number(el.dataset.i); renderPalette(); };
    });
  }

  function pickPalette(i) {
    const hit = S.paletteHits[i];
    if (!hit) return;
    closePalette();
    openDoc(hit.path);
  }

  // ---------------------------------------------------------------- bind

  function bindChrome() {
    $("#vault-btn").onclick = () => {
      if (!S.vault) return pickVault();
      const box = modal(`<h3>仓库</h3>
        <div class="batch-line" style="word-break:break-all">${esc(S.vault.root)}</div>
        <div style="margin-top:12px;font-size:13px;color:var(--text-dim)">${S.vault.docs} 个文档 · ${S.vault.cards} 张卡片</div>
        <div class="row-end">
          <button class="tool-btn" id="v-close">关闭</button>
          <button class="tool-btn" id="v-reveal">在文件管理器显示</button>
          <button class="tool-btn primary" id="v-pick">换一个文件夹</button>
        </div>`);
      box.querySelector("#v-close").onclick = closeModal;
      box.querySelector("#v-reveal").onclick = () => invoke("reveal", { path: S.vault.root });
      box.querySelector("#v-pick").onclick = () => { closeModal(); pickVault(); };
    };

    $("#onboard-pick").onclick = pickVault;
    const onboardFile = $("#onboard-file");
    if (onboardFile) onboardFile.onclick = openSingleFile;

    $$(".side-tab").forEach((tab) => {
      tab.onclick = () => {
        $$(".side-tab").forEach((t) => t.classList.toggle("active", t === tab));
        $("#files-pane").classList.toggle("hidden", tab.dataset.tab !== "files");
        $("#cards-pane").classList.toggle("hidden", tab.dataset.tab !== "cards");
      };
    });

    $("#new-doc").onclick = () => newDocModal();
    $("#open-file").onclick = openSingleFile;
    $("#new-card").onclick = newCardModal;
    $("#search-btn").onclick = openPalette;
    $("#card-filter-kind").onchange = renderCardList;

    $("#mode-write").onclick = () => { setMode("write"); renderPreview(); };
    $("#mode-cards").onclick = () => { setMode("cards"); renderCardsBody(); };
    $$("[data-cardview]").forEach((b) => { b.onclick = () => setCardView(b.dataset.cardview); });

    $("#btn-save").onclick = () => saveCurrent(false);
    $("#btn-export").onclick = exportCurrent;
    $("#btn-batch").onclick = batchExport;
    $("#btn-cardbook").onclick = exportCardbook;
    $("#btn-settings").onclick = () => {
      $("#settings-drawer").classList.remove("hidden");
      renderSettings();
    };
    $("#settings-close").onclick = () => $("#settings-drawer").classList.add("hidden");
    $("#settings-done").onclick = () => $("#settings-drawer").classList.add("hidden");
    $("#btn-copy-typ").onclick = async () => {
      try {
        const src = await invoke("typst_source", {
          path: S.doc?.path || null,
          content: S.mode === "cards" && $("#card-form") ? composeCardMarkdown() : editorValue(),
          options: S.options,
        });
        await copyText(src);
        toast("Typst 源码已复制", "ok");
      } catch (e) { toast(String(e), "error"); }
    };

    $("#btn-theme").onclick = () => {
      S.dark = !S.dark;
      applyTheme();
      savePrefs();
      S.imgCache.clear();
      if (S.mode === "write") renderPreview(); else renderCardPreview();
    };

    $("#mobile-menu").onclick = () => $("#sidebar").classList.toggle("open");

    // tree interactions
    $("#file-tree").addEventListener("click", async (e) => {
      const action = e.target.closest("button[data-act]");
      if (action) {
        e.stopPropagation();
        const path = action.dataset.path;
        const act = action.dataset.act;
        if (act === "new") return newDocModal(path);
        if (act === "rename") {
          const name = prompt("新文件名（不含 .md）", baseName(path));
          if (!name) return;
          const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
          const to = `${dir ? dir + "/" : ""}${name}.md`;
          try {
            const doc = await invoke("rename_entry", { from: path, to });
            await refreshAll();
            openDocDto(doc);
          } catch (err) { toast(String(err), "error"); }
          return;
        }
        if (act === "delete") {
          return confirmModal(`把「${baseName(path)}」移到回收站？`, async () => {
            await invoke("delete_entry", { path });
            if (S.activePath === path) { S.doc = null; S.activePath = null; setEditorValue(""); }
            await refreshAll();
            toast("已移入回收站", "ok");
          });
        }
      }
      const row = e.target.closest(".row[data-path]");
      if (!row) return;
      const path = row.dataset.path;
      if (row.dataset.dir === "true") {
        if (S.collapsed.has(path)) S.collapsed.delete(path); else S.collapsed.add(path);
        renderTree();
      } else {
        openDoc(path);
      }
    });

    $("#recent-list").addEventListener("click", (e) => {
      const row = e.target.closest("[data-open]");
      if (row) openDoc(row.dataset.open);
    });

    $("#card-list").addEventListener("click", (e) => {
      const item = e.target.closest("[data-card-path]");
      if (item) openDoc(item.dataset.cardPath);
    });

    // palette
    $("#palette").addEventListener("click", (e) => {
      if (e.target.id === "palette") closePalette();
    });
    $("#palette-input").addEventListener("input", (e) => {
      clearTimeout(paletteTimer);
      paletteTimer = setTimeout(() => runSearch(e.target.value), 160);
    });
    $("#palette-input").addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        S.paletteSel = Math.min(S.paletteHits.length - 1, S.paletteSel + 1);
        renderPalette();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        S.paletteSel = Math.max(0, S.paletteSel - 1);
        renderPalette();
      } else if (e.key === "Enter") {
        e.preventDefault();
        pickPalette(S.paletteSel);
      } else if (e.key === "Escape") closePalette();
    });

    $("#modal").addEventListener("click", (e) => {
      if (e.target.id === "modal") closeModal();
    });

    // divider
    const divider = $("#divider");
    let dragging = false;
    divider.addEventListener("mousedown", () => {
      dragging = true;
      document.body.style.cursor = "col-resize";
    });
    window.addEventListener("mousemove", (e) => {
      if (!dragging) return;
      const area = $("#write-view").getBoundingClientRect();
      const ratio = Math.min(0.78, Math.max(0.22, (e.clientX - area.left) / area.width));
      $("#editor-pane").style.flex = `0 0 ${ratio * 100}%`;
      $("#preview-pane").style.flex = "1 1 auto";
      S.split = ratio;
    });
    window.addEventListener("mouseup", () => {
      if (!dragging) return;
      dragging = false;
      document.body.style.cursor = "";
      savePrefs();
      applyZoom();
    });
  }

  function bindKeys() {
    window.addEventListener("keydown", (e) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); saveCurrent(false); }
      else if (mod && e.key.toLowerCase() === "e") { e.preventDefault(); exportCurrent(); }
      else if (mod && e.key.toLowerCase() === "k") { e.preventDefault(); openPalette(); }
      else if (mod && e.key.toLowerCase() === "n") { e.preventDefault(); newDocModal(); }
      else if (mod && e.key.toLowerCase() === "p") {
        e.preventDefault();
        const pane = $("#preview-pane");
        pane.classList.toggle("hidden");
        applyZoom();
      } else if (e.key === "Escape") {
        closePalette();
        closeModal();
        $("#settings-drawer").classList.add("hidden");
      }
    });
  }

  function bindDrop() {
    if (!evt) return;
    const overlay = $("#drop-overlay");
    evt.listen("tauri://drag-enter", () => overlay.classList.remove("hidden"));
    evt.listen("tauri://drag-leave", () => overlay.classList.add("hidden"));
    evt.listen("tauri://drag-drop", async (e) => {
      overlay.classList.add("hidden");
      const paths = (e.payload && e.payload.paths) || [];
      for (const p of paths) {
        const norm = String(p).replace(/\\/g, "/");
        if (norm.endsWith(".md") || norm.endsWith(".markdown")) {
          if (S.vault && norm.startsWith(S.vault.root.replace(/\\/g, "/") + "/")) {
            await openDoc(norm.slice(S.vault.root.length + 1));
          } else {
            try {
              const doc = await invoke("open_file", { path: p });
              await afterOpenFile(doc);
            } catch {
              toast("无法打开该文件", "error");
            }
          }
        } else {
          try { await openVault(p); } catch { toast("无法把该路径作为仓库打开", "error"); }
        }
      }
    });
  }

  window.addEventListener("DOMContentLoaded", boot);
})();
