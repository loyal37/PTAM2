import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

interface TextureInfo {
  id: number;
  name: string;
  path: string;
  width: number;
  height: number;
  format: string;
  thumbnailDataUrl: string;
}

interface LoadFailure {
  path: string;
  message: string;
}

interface AddTexturesResponse {
  textures: TextureInfo[];
  errors: LoadFailure[];
  duplicateCount: number;
}

interface SlotGridInfo {
  slotWidth: number;
  slotHeight: number;
  columns: number;
  rows: number;
}

interface AtlasPlacement {
  textureId: number;
  name: string;
  path: string;
  x: number;
  y: number;
  width: number;
  height: number;
  slot: number | null;
}

interface PreviewResponse {
  width: number;
  height: number;
  previewDataUrl: string;
  placements: AtlasPlacement[];
  grid: SlotGridInfo | null;
}

interface ProjectStateResponse {
  textures: TextureInfo[];
  base: TextureInfo | null;
}

interface SlotDiagnostic {
  slot: number;
  texture: string;
  method: string;
}

interface ExportReport {
  outputPath: string;
  jsonPath: string | null;
  mode: string;
  format: string;
  width: number;
  height: number;
  elapsedMs: number;
  preservedOutsideSlots: boolean;
  diagnostics: SlotDiagnostic[];
}

interface PersistedSettings {
  layoutMode: string;
  columns: number;
  padding: number;
  canvasMode: string;
  canvasWidth: number;
  canvasHeight: number;
  exportFormat: string;
  quality: string;
  exportJson: boolean;
  panelOpacity: number;
  backgroundPath: string | null;
}

const defaultSettings: PersistedSettings = {
  layoutMode: "auto",
  columns: 2,
  padding: 0,
  canvasMode: "auto",
  canvasWidth: 4096,
  canvasHeight: 4096,
  exportFormat: "png",
  quality: "normal",
  exportJson: true,
  panelOpacity: 88,
  backgroundPath: null,
};

function loadSettings(): PersistedSettings {
  try {
    return { ...defaultSettings, ...JSON.parse(localStorage.getItem("ptam2.settings") ?? "{}") };
  } catch {
    return { ...defaultSettings };
  }
}

const state = {
  textures: [] as TextureInfo[],
  base: null as TextureInfo | null,
  selected: new Set<number>(),
  assignments: new Map<number, number>(),
  preview: null as PreviewResponse | null,
  settings: loadSettings(),
  busy: false,
};

type ActiveDrag =
  | { kind: "texture"; textureId: number }
  | { kind: "slot"; sourceSlot: number };

let activeDrag: ActiveDrag | null = null;

document.querySelector<HTMLDivElement>("#app")!.innerHTML = `
  <div class="background-art" id="background-art"></div>
  <main class="app-shell">
    <header class="topbar glass">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true"><span></span><span></span><span></span><span></span></div>
        <div><h1>PTAM<span>2</span></h1><p>Texture Atlas Studio</p></div>
      </div>
      <div class="topbar-status"><span class="status-dot"></span><span id="engine-status">Rust 图像引擎就绪</span></div>
      <div class="top-actions">
        <button class="ghost-button" id="background-button" title="设置界面背景">背景</button>
        <button class="ghost-button danger-subtle" id="clear-background-button" title="清除界面背景">清除背景</button>
      </div>
    </header>

    <section class="workspace">
      <aside class="control-column glass">
        <div class="section-heading">
          <div><span class="eyebrow">INPUT</span><h2>贴图库</h2></div>
          <span class="count-badge" id="texture-count">0</span>
        </div>
        <div class="button-grid four-actions">
          <button class="primary-button" id="add-button"><span>＋</span>添加</button>
          <button id="remove-button">移除</button>
          <button id="resize-button">统一尺寸</button>
          <button class="danger-button" id="clear-button">清空</button>
        </div>
        <div class="texture-list" id="texture-list">
          <div class="list-empty"><div class="empty-icon">◇</div><strong>尚未添加贴图</strong><span>支持 DDS / PNG / JPG / BMP / TGA / TIFF / WebP</span></div>
        </div>

        <div class="section-divider"></div>
        <div class="section-heading compact">
          <div><span class="eyebrow">BASE MODE</span><h2>底图槽位</h2></div>
        </div>
        <div class="button-grid">
          <button id="base-button">选择底图</button>
          <button class="danger-button" id="clear-base-button">清除底图</button>
        </div>
        <div class="base-card" id="base-card">
          <div class="base-placeholder">未启用底图模式</div>
        </div>
        <div class="slot-summary" id="slot-summary">添加底图与贴图后可拖拽分配槽位</div>
        <button class="wide-button" id="auto-place-button">自动顺序放置</button>

        <div class="section-divider"></div>
        <div class="section-heading compact">
          <div><span class="eyebrow">LAYOUT</span><h2>拼图设置</h2></div>
        </div>
        <div class="settings-grid">
          <label>排列方式<select id="layout-mode">
            <option value="auto">自动网格</option><option value="horizontal">横向</option>
            <option value="vertical">纵向</option><option value="grid">固定列数</option>
          </select></label>
          <label id="columns-field">固定列数<input id="columns" type="number" min="1" max="64" /></label>
          <label>间距像素<input id="padding" type="number" min="0" max="512" /></label>
          <label>最终画布<select id="canvas-mode">
            <option value="auto">自动</option><option value="1024">1024 × 1024</option>
            <option value="2048">2048 × 2048</option><option value="4096">4096 × 4096</option>
            <option value="8192">8192 × 8192</option><option value="custom">自定义</option>
          </select></label>
          <div class="size-pair" id="custom-size-fields">
            <label>宽度<input id="canvas-width" type="number" min="1" max="32768" /></label>
            <span>×</span>
            <label>高度<input id="canvas-height" type="number" min="1" max="32768" /></label>
          </div>
          <label>导出格式<select id="export-format">
            <option value="png">PNG 无损</option><option value="dxt5">DDS · DXT5</option>
            <option value="bc7-linear">DDS · BC7 线性</option><option value="bc7-srgb">DDS · BC7 sRGB</option>
          </select></label>
          <label id="quality-field">压缩质量<select id="quality">
            <option value="fast">快速</option><option value="normal">均衡</option><option value="slow">高质量</option>
          </select></label>
        </div>
        <label class="check-row"><input id="export-json" type="checkbox" /><span>同时导出 JSON 坐标表</span></label>
        <label class="opacity-row"><span>面板不透明度</span><input id="panel-opacity" type="range" min="45" max="100" /></label>
      </aside>

      <section class="preview-column glass">
        <div class="preview-header">
          <div><span class="eyebrow">LIVE CANVAS</span><h2>图集预览</h2></div>
          <div class="canvas-meta"><span id="canvas-dimensions">— × —</span><span id="canvas-mode-badge">等待生成</span></div>
        </div>
        <div class="preview-stage" id="preview-stage">
          <div class="stage-empty" id="stage-empty">
            <div class="stage-orbit"><span></span><i></i></div>
            <h3>准备创建你的图集</h3>
            <p>添加贴图并生成预览；使用 DDS 底图时，可把左侧贴图拖入槽位。</p>
          </div>
          <div class="atlas-frame" id="atlas-frame" hidden>
            <img id="preview-image" alt="图集预览" draggable="false" />
            <div class="slot-overlay" id="slot-overlay"></div>
          </div>
          <div class="busy-overlay" id="busy-overlay" hidden><span class="spinner"></span><strong id="busy-label">正在处理</strong></div>
        </div>
        <div class="preview-footer">
          <div class="legend">
            <span><i class="legend-box available"></i>可用槽位</span>
            <span><i class="legend-box occupied"></i>已放置</span>
            <span><i class="legend-box patch"></i>DDS 原位补丁</span>
          </div>
          <div class="footer-actions">
            <button class="large-button" id="preview-button">生成预览</button>
            <button class="large-button accent-button" id="export-button">导出图集 <span>→</span></button>
          </div>
        </div>
        <div class="diagnostic-strip" id="diagnostic-strip">
          <span class="diagnostic-icon">i</span>
          <p><strong>原生处理管线</strong><span>预览最长边限制为 2048px，导出始终使用完整分辨率。</span></p>
        </div>
      </section>
    </section>
  </main>
  <div class="toast-stack" id="toast-stack"></div>
  <div class="modal-root" id="modal-root"></div>
`;

const $ = <T extends HTMLElement>(selector: string): T => {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing element ${selector}`);
  return element;
};

const textureList = $("#texture-list");
const previewStage = $("#preview-stage");
const atlasFrame = $("#atlas-frame");
const slotOverlay = $("#slot-overlay");
const previewImage = $("#preview-image") as HTMLImageElement;
const modalRoot = $("#modal-root");

function persistSettings(): void {
  localStorage.setItem("ptam2.settings", JSON.stringify(state.settings));
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
  })[character] ?? character);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function toast(message: string, kind: "success" | "error" | "info" = "info", duration = 4200): void {
  const item = document.createElement("div");
  item.className = `toast ${kind}`;
  item.innerHTML = `<span>${kind === "success" ? "✓" : kind === "error" ? "!" : "i"}</span><p>${escapeHtml(message)}</p>`;
  $("#toast-stack").append(item);
  requestAnimationFrame(() => item.classList.add("visible"));
  window.setTimeout(() => {
    item.classList.remove("visible");
    window.setTimeout(() => item.remove(), 240);
  }, duration);
}

function setBusy(busy: boolean, label = "正在处理"): void {
  state.busy = busy;
  const overlay = $("#busy-overlay");
  overlay.toggleAttribute("hidden", !busy);
  $("#busy-label").textContent = label;
  document.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
    if (button.id !== "clear-background-button") button.disabled = busy;
  });
  if (!busy) updateButtonState();
}

function invalidatePreview(): void {
  state.preview = null;
  renderPreview();
}

function buildOptions() {
  let canvasWidth: number | null = null;
  let canvasHeight: number | null = null;
  if (state.settings.canvasMode === "custom") {
    canvasWidth = state.settings.canvasWidth;
    canvasHeight = state.settings.canvasHeight;
  } else if (state.settings.canvasMode !== "auto") {
    canvasWidth = Number(state.settings.canvasMode);
    canvasHeight = Number(state.settings.canvasMode);
  }
  return {
    layoutMode: state.settings.layoutMode,
    padding: state.settings.padding,
    columns: state.settings.columns,
    canvasWidth,
    canvasHeight,
    assignments: Array.from(state.assignments, ([textureId, slot]) => ({ textureId, slot })),
  };
}

function renderTextures(): void {
  $("#texture-count").textContent = String(state.textures.length);
  if (state.textures.length === 0) {
    textureList.innerHTML = `<div class="list-empty"><div class="empty-icon">◇</div><strong>尚未添加贴图</strong><span>支持 DDS / PNG / JPG / BMP / TGA / TIFF / WebP</span></div>`;
  } else {
    textureList.innerHTML = state.textures.map((texture, index) => `
      <article class="texture-item ${state.selected.has(texture.id) ? "selected" : ""}" data-id="${texture.id}" draggable="${Boolean(state.base)}">
        <div class="texture-index">${String(index + 1).padStart(2, "0")}</div>
        <img src="${texture.thumbnailDataUrl}" alt="" />
        <div class="texture-copy"><strong title="${escapeHtml(texture.path)}">${escapeHtml(texture.name)}</strong><span>${texture.width} × ${texture.height}</span></div>
        <span class="format-badge">${escapeHtml(texture.format)}</span>
        ${state.assignments.has(texture.id) ? `<span class="slot-pill">槽 ${state.assignments.get(texture.id)}</span>` : ""}
      </article>
    `).join("");
  }
  updateButtonState();
}

function renderBase(): void {
  const card = $("#base-card");
  if (!state.base) {
    card.innerHTML = `<div class="base-placeholder">未启用底图模式</div>`;
  } else {
    card.innerHTML = `<img src="${state.base.thumbnailDataUrl}" alt="" /><div><strong>${escapeHtml(state.base.name)}</strong><span>${state.base.width} × ${state.base.height} · ${escapeHtml(state.base.format)}</span></div><span class="active-chip">ACTIVE</span>`;
  }
  renderSlotSummary();
  updateButtonState();
}

function inferredGrid(): SlotGridInfo | null {
  if (!state.base || state.textures.length === 0) return null;
  const first = state.textures[0];
  if (state.textures.some((texture) => texture.width !== first.width || texture.height !== first.height)) return null;
  if (state.base.width % first.width !== 0 || state.base.height % first.height !== 0) return null;
  return {
    slotWidth: first.width,
    slotHeight: first.height,
    columns: state.base.width / first.width,
    rows: state.base.height / first.height,
  };
}

function renderSlotSummary(): void {
  const summary = $("#slot-summary");
  if (!state.base) {
    summary.textContent = "添加底图与贴图后可拖拽分配槽位";
    summary.className = "slot-summary";
    return;
  }
  if (state.textures.length === 0) {
    summary.textContent = "底图已加载，请添加待合并贴图";
    summary.className = "slot-summary";
    return;
  }
  const grid = inferredGrid();
  if (!grid) {
    summary.textContent = "贴图尺寸不一致，或无法整齐划分底图";
    summary.className = "slot-summary warning";
    return;
  }
  summary.textContent = `${grid.columns} × ${grid.rows} 网格 · 已放置 ${state.assignments.size}/${state.textures.length}`;
  summary.className = state.assignments.size === state.textures.length ? "slot-summary ready" : "slot-summary";
}

function renderPreview(): void {
  const preview = state.preview;
  $("#stage-empty").toggleAttribute("hidden", Boolean(preview));
  atlasFrame.toggleAttribute("hidden", !preview);
  if (!preview) {
    $("#canvas-dimensions").textContent = "— × —";
    $("#canvas-mode-badge").textContent = "等待生成";
    slotOverlay.innerHTML = "";
    return;
  }
  previewImage.src = preview.previewDataUrl;
  $("#canvas-dimensions").textContent = `${preview.width} × ${preview.height}`;
  $("#canvas-mode-badge").textContent = preview.grid ? "槽位模式" : "图集模式";
  fitAtlasFrame();
  renderSlots();
}

function fitAtlasFrame(): void {
  if (!state.preview) return;
  const width = Math.max(120, previewStage.clientWidth - 72);
  const height = Math.max(120, previewStage.clientHeight - 72);
  const scale = Math.min(width / state.preview.width, height / state.preview.height);
  atlasFrame.style.width = `${Math.max(1, Math.floor(state.preview.width * scale))}px`;
  atlasFrame.style.height = `${Math.max(1, Math.floor(state.preview.height * scale))}px`;
}

function renderSlots(): void {
  const preview = state.preview;
  if (!preview?.grid) {
    slotOverlay.innerHTML = "";
    return;
  }
  const grid = preview.grid;
  const occupied = new Map<number, TextureInfo>();
  for (const [textureId, slot] of state.assignments) {
    const texture = state.textures.find((item) => item.id === textureId);
    if (texture) occupied.set(slot, texture);
  }
  const cells: string[] = [];
  for (let slot = 1; slot <= grid.columns * grid.rows; slot += 1) {
    const zero = slot - 1;
    const column = zero % grid.columns;
    const row = Math.floor(zero / grid.columns);
    const texture = occupied.get(slot);
    const left = column * grid.slotWidth / preview.width * 100;
    const top = row * grid.slotHeight / preview.height * 100;
    const width = grid.slotWidth / preview.width * 100;
    const height = grid.slotHeight / preview.height * 100;
    cells.push(`<div class="slot-cell ${texture ? "occupied" : ""}" data-slot="${slot}" draggable="${Boolean(texture)}" style="left:${left}%;top:${top}%;width:${width}%;height:${height}%">
      <span>${slot}</span>${texture ? `<strong>${escapeHtml(texture.name)}</strong>` : ""}
    </div>`);
  }
  slotOverlay.innerHTML = cells.join("");
}

function updateButtonState(): void {
  const disabled = state.busy;
  ($("#remove-button") as HTMLButtonElement).disabled = disabled || state.selected.size === 0;
  ($("#resize-button") as HTMLButtonElement).disabled = disabled || state.textures.length === 0;
  ($("#clear-button") as HTMLButtonElement).disabled = disabled || state.textures.length === 0;
  ($("#clear-base-button") as HTMLButtonElement).disabled = disabled || !state.base;
  ($("#auto-place-button") as HTMLButtonElement).disabled = disabled || !inferredGrid() || state.textures.length === 0;
  ($("#preview-button") as HTMLButtonElement).disabled = disabled || (state.textures.length === 0 && !state.base);
  ($("#export-button") as HTMLButtonElement).disabled = disabled || (state.textures.length === 0 && !state.base);
  ($("#clear-background-button") as HTMLButtonElement).disabled = disabled || !state.settings.backgroundPath;
}

function applySettingsToUi(): void {
  const settings = state.settings;
  ($("#layout-mode") as HTMLSelectElement).value = settings.layoutMode;
  ($("#columns") as HTMLInputElement).value = String(settings.columns);
  ($("#padding") as HTMLInputElement).value = String(settings.padding);
  ($("#canvas-mode") as HTMLSelectElement).value = settings.canvasMode;
  ($("#canvas-width") as HTMLInputElement).value = String(settings.canvasWidth);
  ($("#canvas-height") as HTMLInputElement).value = String(settings.canvasHeight);
  ($("#export-format") as HTMLSelectElement).value = settings.exportFormat;
  ($("#quality") as HTMLSelectElement).value = settings.quality;
  ($("#export-json") as HTMLInputElement).checked = settings.exportJson;
  ($("#panel-opacity") as HTMLInputElement).value = String(settings.panelOpacity);
  document.documentElement.style.setProperty("--panel-alpha", String(settings.panelOpacity / 100));
  updateConditionalSettings();
}

function updateConditionalSettings(): void {
  $("#columns-field").classList.toggle("muted-field", state.settings.layoutMode !== "grid");
  ($("#columns") as HTMLInputElement).disabled = state.settings.layoutMode !== "grid";
  $("#custom-size-fields").classList.toggle("visible", state.settings.canvasMode === "custom");
  $("#quality-field").classList.toggle("muted-field", state.settings.exportFormat === "png");
  ($("#quality") as HTMLSelectElement).disabled = state.settings.exportFormat === "png";
}

async function generatePreview(silent = false): Promise<void> {
  if (state.textures.length === 0 && !state.base) return;
  setBusy(true, "Rust 引擎正在生成预览");
  try {
    state.preview = await invoke<PreviewResponse>("build_preview", { options: buildOptions() });
    renderPreview();
    if (!silent) toast(`预览已生成：${state.preview.width} × ${state.preview.height}`, "success");
  } catch (error) {
    toast(errorText(error), "error", 6500);
  } finally {
    setBusy(false);
  }
}

async function chooseTextures(): Promise<void> {
  const paths = await open({
    multiple: true,
    filters: [{ name: "贴图文件", extensions: ["dds", "png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp"] }],
  });
  if (!paths) return;
  setBusy(true, "正在解码贴图");
  try {
    const response = await invoke<AddTexturesResponse>("add_textures", { paths: Array.isArray(paths) ? paths : [paths] });
    state.textures.push(...response.textures);
    state.assignments.clear();
    state.selected.clear();
    invalidatePreview();
    renderTextures();
    renderSlotSummary();
    if (response.textures.length) toast(`已添加 ${response.textures.length} 张贴图`, "success");
    if (response.duplicateCount) toast(`已忽略 ${response.duplicateCount} 个重复文件`, "info");
    if (response.errors.length) {
      toast(response.errors.map((item) => `${item.path}: ${item.message}`).join("\n"), "error", 8000);
    }
    if (state.base && response.textures.length) await generatePreview(true);
  } catch (error) {
    toast(errorText(error), "error");
  } finally {
    setBusy(false);
  }
}

async function removeSelected(): Promise<void> {
  if (!state.selected.size) return;
  const ids = Array.from(state.selected);
  try {
    await invoke("remove_textures", { ids });
    state.textures = state.textures.filter((texture) => !state.selected.has(texture.id));
    ids.forEach((id) => state.assignments.delete(id));
    state.selected.clear();
    state.assignments.clear();
    invalidatePreview();
    renderTextures();
    renderSlotSummary();
  } catch (error) {
    toast(errorText(error), "error");
  }
}

async function clearTextures(): Promise<void> {
  if (!state.textures.length) return;
  try {
    await invoke("clear_textures");
    state.textures = [];
    state.selected.clear();
    state.assignments.clear();
    invalidatePreview();
    renderTextures();
    renderSlotSummary();
  } catch (error) {
    toast(errorText(error), "error");
  }
}

async function chooseBase(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: "底图文件", extensions: ["dds", "png", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "webp"] }],
  });
  if (!path || Array.isArray(path)) return;
  setBusy(true, "正在解码底图");
  try {
    state.base = await invoke<TextureInfo>("set_base_texture", { path });
    state.assignments.clear();
    invalidatePreview();
    renderBase();
    renderTextures();
    toast("底图模式已启用", "success");
    if (state.textures.length) await generatePreview(true);
  } catch (error) {
    toast(errorText(error), "error");
  } finally {
    setBusy(false);
  }
}

async function clearBase(): Promise<void> {
  try {
    await invoke("clear_base_texture");
    state.base = null;
    state.assignments.clear();
    invalidatePreview();
    renderBase();
    renderTextures();
  } catch (error) {
    toast(errorText(error), "error");
  }
}

function autoPlace(): void {
  const grid = inferredGrid();
  if (!grid) return;
  if (state.textures.length > grid.columns * grid.rows) {
    toast("贴图数量超过可用槽位数", "error");
    return;
  }
  state.assignments.clear();
  state.textures.forEach((texture, index) => state.assignments.set(texture.id, index + 1));
  renderTextures();
  renderSlotSummary();
  void generatePreview(true);
}

function assignTexture(textureId: number, targetSlot: number): void {
  for (const [id, slot] of state.assignments) {
    if (slot === targetSlot && id !== textureId) state.assignments.delete(id);
  }
  state.assignments.set(textureId, targetSlot);
  renderTextures();
  renderSlotSummary();
  void generatePreview(true);
}

function moveSlot(sourceSlot: number, targetSlot: number): void {
  if (sourceSlot === targetSlot) return;
  const source = Array.from(state.assignments).find(([, slot]) => slot === sourceSlot);
  if (!source) return;
  const target = Array.from(state.assignments).find(([, slot]) => slot === targetSlot);
  state.assignments.set(source[0], targetSlot);
  if (target) state.assignments.set(target[0], sourceSlot);
  renderTextures();
  renderSlotSummary();
  void generatePreview(true);
}

function showResizeDialog(): void {
  if (!state.textures.length) return;
  const first = state.textures[0];
  modalRoot.innerHTML = `<div class="modal-backdrop"><div class="modal-card compact-modal">
    <span class="eyebrow">RESIZE</span><h2>统一贴图尺寸</h2>
    <p>将使用 Lanczos3 高质量重采样处理 ${state.textures.length} 张贴图。</p>
    <div class="modal-size-row"><label>宽度<input id="resize-width" type="number" min="1" max="32768" value="${first.width}" /></label><span>×</span><label>高度<input id="resize-height" type="number" min="1" max="32768" value="${first.height}" /></label></div>
    <label class="check-row"><input id="lock-aspect" type="checkbox" checked /><span>锁定第一张贴图的宽高比</span></label>
    <div class="modal-actions"><button data-close>取消</button><button class="accent-button" id="confirm-resize">应用尺寸</button></div>
  </div></div>`;
  const widthInput = $("#resize-width") as HTMLInputElement;
  const heightInput = $("#resize-height") as HTMLInputElement;
  const lock = $("#lock-aspect") as HTMLInputElement;
  const aspect = first.width / first.height;
  widthInput.addEventListener("input", () => {
    if (lock.checked) heightInput.value = String(Math.max(1, Math.round(Number(widthInput.value) / aspect)));
  });
  heightInput.addEventListener("input", () => {
    if (lock.checked) widthInput.value = String(Math.max(1, Math.round(Number(heightInput.value) * aspect)));
  });
  modalRoot.querySelector("[data-close]")?.addEventListener("click", closeModal);
  $("#confirm-resize").addEventListener("click", async () => {
    const width = Number(widthInput.value);
    const height = Number(heightInput.value);
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1) {
      toast("请输入有效的宽高", "error");
      return;
    }
    closeModal();
    setBusy(true, "正在重采样贴图");
    try {
      state.textures = await invoke<TextureInfo[]>("resize_textures", { width, height });
      state.assignments.clear();
      invalidatePreview();
      renderTextures();
      renderSlotSummary();
      toast(`已统一为 ${width} × ${height}`, "success");
    } catch (error) {
      toast(errorText(error), "error");
    } finally {
      setBusy(false);
    }
  });
}

function closeModal(): void {
  modalRoot.innerHTML = "";
}

async function exportAtlas(): Promise<void> {
  const isPng = state.settings.exportFormat === "png";
  const path = await save({
    defaultPath: isPng ? "atlas.png" : "atlas.dds",
    filters: [{ name: isPng ? "PNG 图像" : "DDS 贴图", extensions: [isPng ? "png" : "dds"] }],
  });
  if (!path) return;
  setBusy(true, isPng ? "正在写入 PNG" : "正在压缩 DDS");
  try {
    const report = await invoke<ExportReport>("export_atlas", {
      request: {
        outputPath: path,
        format: state.settings.exportFormat,
        quality: state.settings.quality,
        exportJson: state.settings.exportJson,
        options: buildOptions(),
      },
    });
    showExportReport(report);
    toast("图集导出完成", "success");
  } catch (error) {
    toast(errorText(error), "error", 8000);
  } finally {
    setBusy(false);
  }
}

function showExportReport(report: ExportReport): void {
  const patchMode = report.mode === "binary-patch";
  const diagnostics = report.diagnostics.length
    ? `<div class="diagnostic-table">${report.diagnostics.map((item) => `<div><span>槽 ${item.slot}</span><strong>${escapeHtml(item.texture)}</strong><code>${item.method}</code></div>`).join("")}</div>`
    : "";
  modalRoot.innerHTML = `<div class="modal-backdrop"><div class="modal-card report-modal">
    <div class="report-symbol ${patchMode ? "patch" : "success"}">${patchMode ? "BC" : "✓"}</div>
    <span class="eyebrow">EXPORT COMPLETE</span><h2>图集已安全写入</h2>
    <div class="report-grid"><div><span>尺寸</span><strong>${report.width} × ${report.height}</strong></div><div><span>格式</span><strong>${escapeHtml(report.format)}</strong></div><div><span>模式</span><strong>${escapeHtml(report.mode)}</strong></div><div><span>耗时</span><strong>${report.elapsedMs} ms</strong></div></div>
    ${patchMode ? `<p class="preserve-note">${report.preservedOutsideSlots ? "✓ 已验证：目标槽位之外的原始 DDS 字节完全不变" : "未执行字节保护验证"}</p>` : ""}
    <div class="path-block"><span>输出文件</span><code>${escapeHtml(report.outputPath)}</code>${report.jsonPath ? `<span>坐标表</span><code>${escapeHtml(report.jsonPath)}</code>` : ""}</div>
    ${diagnostics}
    <div class="modal-actions"><button class="accent-button" data-close>完成</button></div>
  </div></div>`;
  modalRoot.querySelector("[data-close]")?.addEventListener("click", closeModal);
}

async function chooseBackground(): Promise<void> {
  const path = await open({ multiple: false, filters: [{ name: "背景图", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"] }] });
  if (!path || Array.isArray(path)) return;
  try {
    const dataUrl = await invoke<string>("read_background_image", { path });
    $("#background-art").style.backgroundImage = `linear-gradient(rgba(3, 9, 18, .40), rgba(3, 9, 18, .72)), url("${dataUrl}")`;
    state.settings.backgroundPath = path;
    persistSettings();
    updateButtonState();
  } catch (error) {
    toast(errorText(error), "error");
  }
}

function clearBackground(): void {
  state.settings.backgroundPath = null;
  $("#background-art").style.backgroundImage = "";
  persistSettings();
  updateButtonState();
}

async function restoreBackground(): Promise<void> {
  if (!state.settings.backgroundPath) return;
  try {
    const dataUrl = await invoke<string>("read_background_image", { path: state.settings.backgroundPath });
    $("#background-art").style.backgroundImage = `linear-gradient(rgba(3, 9, 18, .40), rgba(3, 9, 18, .72)), url("${dataUrl}")`;
  } catch {
    state.settings.backgroundPath = null;
    persistSettings();
  }
}

function wireEvents(): void {
  $("#add-button").addEventListener("click", () => void chooseTextures());
  $("#remove-button").addEventListener("click", () => void removeSelected());
  $("#clear-button").addEventListener("click", () => void clearTextures());
  $("#resize-button").addEventListener("click", showResizeDialog);
  $("#base-button").addEventListener("click", () => void chooseBase());
  $("#clear-base-button").addEventListener("click", () => void clearBase());
  $("#auto-place-button").addEventListener("click", autoPlace);
  $("#preview-button").addEventListener("click", () => void generatePreview());
  $("#export-button").addEventListener("click", () => void exportAtlas());
  $("#background-button").addEventListener("click", () => void chooseBackground());
  $("#clear-background-button").addEventListener("click", clearBackground);

  textureList.addEventListener("click", (event) => {
    const item = (event.target as HTMLElement).closest<HTMLElement>(".texture-item");
    if (!item) return;
    const id = Number(item.dataset.id);
    if ((event as MouseEvent).ctrlKey || (event as MouseEvent).metaKey) {
      state.selected.has(id) ? state.selected.delete(id) : state.selected.add(id);
    } else {
      const onlyThis = state.selected.size === 1 && state.selected.has(id);
      state.selected.clear();
      if (!onlyThis) state.selected.add(id);
    }
    renderTextures();
  });
  textureList.addEventListener("dragstart", (event) => {
    const item = (event.target as HTMLElement).closest<HTMLElement>(".texture-item");
    if (!item || !event.dataTransfer) return;
    const textureId = Number(item.dataset.id);
    if (!textureId) return;
    activeDrag = { kind: "texture", textureId };
    item.classList.add("dragging");
    event.dataTransfer.setData("application/x-ptam-texture", String(textureId));
    event.dataTransfer.setData("text/plain", `ptam-texture:${textureId}`);
    event.dataTransfer.effectAllowed = "move";
  });
  textureList.addEventListener("dragend", (event) => {
    (event.target as HTMLElement).closest<HTMLElement>(".texture-item")?.classList.remove("dragging");
    activeDrag = null;
    slotOverlay.querySelectorAll(".drag-over").forEach((element) => element.classList.remove("drag-over"));
  });
  slotOverlay.addEventListener("dragstart", (event) => {
    const cell = (event.target as HTMLElement).closest<HTMLElement>(".slot-cell.occupied");
    if (!cell || !event.dataTransfer) return;
    const sourceSlot = Number(cell.dataset.slot);
    if (!sourceSlot) return;
    activeDrag = { kind: "slot", sourceSlot };
    cell.classList.add("dragging");
    event.dataTransfer.setData("application/x-ptam-slot", String(sourceSlot));
    event.dataTransfer.setData("text/plain", `ptam-slot:${sourceSlot}`);
    event.dataTransfer.effectAllowed = "move";
  });
  slotOverlay.addEventListener("dragend", (event) => {
    (event.target as HTMLElement).closest<HTMLElement>(".slot-cell")?.classList.remove("dragging");
    activeDrag = null;
    slotOverlay.querySelectorAll(".drag-over").forEach((element) => element.classList.remove("drag-over"));
  });
  slotOverlay.addEventListener("dragover", (event) => {
    const cell = (event.target as HTMLElement).closest<HTMLElement>(".slot-cell");
    if (!cell) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    slotOverlay.querySelectorAll(".drag-over").forEach((element) => {
      if (element !== cell) element.classList.remove("drag-over");
    });
    cell.classList.add("drag-over");
  });
  slotOverlay.addEventListener("dragleave", (event) => {
    (event.target as HTMLElement).closest<HTMLElement>(".slot-cell")?.classList.remove("drag-over");
  });
  slotOverlay.addEventListener("drop", (event) => {
    const cell = (event.target as HTMLElement).closest<HTMLElement>(".slot-cell");
    if (!cell || !event.dataTransfer) return;
    event.preventDefault();
    cell.classList.remove("drag-over");
    const targetSlot = Number(cell.dataset.slot);
    let textureId = Number(event.dataTransfer.getData("application/x-ptam-texture"));
    let sourceSlot = Number(event.dataTransfer.getData("application/x-ptam-slot"));
    const plain = event.dataTransfer.getData("text/plain");
    if (!textureId && plain.startsWith("ptam-texture:")) textureId = Number(plain.slice(13));
    if (!sourceSlot && plain.startsWith("ptam-slot:")) sourceSlot = Number(plain.slice(10));
    if (!textureId && activeDrag?.kind === "texture") textureId = activeDrag.textureId;
    if (!sourceSlot && activeDrag?.kind === "slot") sourceSlot = activeDrag.sourceSlot;
    activeDrag = null;
    if (textureId) assignTexture(textureId, targetSlot);
    else if (sourceSlot) moveSlot(sourceSlot, targetSlot);
  });

  const bindSetting = (selector: string, apply: (element: HTMLInputElement | HTMLSelectElement) => void) => {
    $(selector).addEventListener("change", (event) => {
      apply(event.target as HTMLInputElement | HTMLSelectElement);
      persistSettings();
      updateConditionalSettings();
      invalidatePreview();
    });
  };
  bindSetting("#layout-mode", (element) => state.settings.layoutMode = element.value);
  bindSetting("#columns", (element) => state.settings.columns = Math.max(1, Number(element.value)));
  bindSetting("#padding", (element) => state.settings.padding = Math.max(0, Number(element.value)));
  bindSetting("#canvas-mode", (element) => state.settings.canvasMode = element.value);
  bindSetting("#canvas-width", (element) => state.settings.canvasWidth = Math.max(1, Number(element.value)));
  bindSetting("#canvas-height", (element) => state.settings.canvasHeight = Math.max(1, Number(element.value)));
  bindSetting("#export-format", (element) => state.settings.exportFormat = element.value);
  bindSetting("#quality", (element) => state.settings.quality = element.value);
  bindSetting("#export-json", (element) => state.settings.exportJson = (element as HTMLInputElement).checked);
  $("#panel-opacity").addEventListener("input", (event) => {
    state.settings.panelOpacity = Number((event.target as HTMLInputElement).value);
    document.documentElement.style.setProperty("--panel-alpha", String(state.settings.panelOpacity / 100));
    persistSettings();
  });
  window.addEventListener("resize", fitAtlasFrame);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeModal();
    if (event.key === "Delete" && state.selected.size) void removeSelected();
  });
}

async function initialize(): Promise<void> {
  applySettingsToUi();
  wireEvents();
  try {
    const project = await invoke<ProjectStateResponse>("get_project_state");
    state.textures = project.textures;
    state.base = project.base;
  } catch (error) {
    toast(`初始化失败：${errorText(error)}`, "error");
  }
  renderTextures();
  renderBase();
  renderPreview();
  await restoreBackground();
}

void initialize();
