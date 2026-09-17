/**
 * 前端唯一的 Tauri 命令调用入口（对应 Rust `src-tauri/src/commands/` 各模块）。
 *
 * 约定：
 * - 每个 `#[tauri::command]` 在此文件**只**有一个对应的 TS 包装函数，命名与 Rust 函数同名转 camelCase。
 * - 调用方一律 `import { foo } from "@/commands"`，**禁止**裸调 `invoke` 或引用 `TAURI_COMMAND` 常量。
 * - 错误处理在本文件统一收口：失败时 log + antd message error toast，
 *   然后再 rethrow。调用方按需用 `try/catch` 决定成功后做什么，**不要再写错误 toast**。
 */

import { invoke } from "@tauri-apps/api/core";
import { TAURI_COMMAND } from "@/constants/commands";
import i18n from "@/i18n";
import { settingsState } from "@/stores/settings";
import type {
  ClipboardAction,
  ClipboardApp,
  ClipboardGroupInput,
  ClipboardGroupRecord,
  ClipboardItem,
  ClipboardItemPage,
  ClipboardItemQuery,
  ClipboardKind,
  ClipboardSubKind,
  PasteTransform,
  UpdateNoteResult,
} from "@/types/clipboard";
import type {
  MergePasteSeparator,
  Settings,
  SettingsPatch,
} from "@/types/settings";
import { getMessageApi, getModalApi } from "@/utils/feedback";
import { log } from "@/utils/log";
import { confirmClearClipboardItems } from "./confirmClearClipboardItems";

/**
 * Rust 端 `AppError` 序列化后的形状：`kind` 用于按变体分流，`message` 给用户看。
 */
interface AppError {
  kind: string;
  message: string;
}

export interface PreviewAnchorRect {
  left: number;
  pointerY?: number;
  top: number;
  width: number;
  height: number;
}

export interface ContextSubmenuAnchor {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface ContextSubmenuGroupInput {
  checked: boolean;
  id: string;
  label: string;
}

export interface ContextMenuItemPayload {
  action: ClipboardAction;
  label: string;
  accelerator: string | null;
  groups?: ContextSubmenuGroupInput[];
}

export interface ContextMenuShowPayload {
  itemId: string;
  isFavorite: boolean;
  isPinned: boolean;
  groups: Array<Array<ContextMenuItemPayload>>;
}

export interface ShowContextSubmenuInput {
  action: ClipboardAction;
  anchor: ContextSubmenuAnchor;
  groups: ContextSubmenuGroupInput[];
  itemId: string;
}

export interface ClipboardPreviewState {
  requestId: number;
  sessionId: number;
  itemId: string;
  anchor: PreviewAnchorRect;
  scaleFactor: number;
  workArea: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  clipboardWindow: {
    x: number;
    y: number;
    width: number;
    height: number;
  } | null;
  layout: ClipboardPreviewLayout;
}

export interface ClipboardPreviewRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export type ClipboardPreviewPlacement = "right" | "left" | "bottom" | "top";

export interface ClipboardPreviewLayout {
  overlayRect: ClipboardPreviewRect;
  sourceRect: ClipboardPreviewRect;
  panelRect: ClipboardPreviewRect;
  placement: ClipboardPreviewPlacement;
}

export interface ClipboardPreviewFileEntry {
  path: string;
  name: string;
  isDir: boolean;
  isImage: boolean;
  exists: boolean;
  size: number | null;
  iconPath?: string;
}

export interface ClipboardPreviewPayload {
  id: string;
  kind: ClipboardKind;
  subKind: ClipboardSubKind | null;
  updatedAt: string;
  text: string | null;
  /** 富文本 HTML（仅 Rich 档位且未脱敏）；预览页 sanitize 后走沙箱 iframe 渲染。 */
  html: string | null;
  /** 预览面板渲染图（960px 预览档路径）；仅灯箱放大用 `imageOriginPath` 原图。 */
  imagePath: string | null;
  imageOriginPath: string | null;
  imageWidth: number | null;
  imageHeight: number | null;
  size: number | null;
  isSensitive: boolean;
  imageExists: boolean;
  files: ClipboardPreviewFileEntry[];
  totalFiles: number;
  /** Files 预览渲染模式：与列表卡片同判据（单文件 + 图片 + 文件存在 → 按图片渲染）。 */
  filesPreviewKind: "imagePreview" | "list";
}

export interface StorageUsage {
  totalBytes: number;
  databaseBytes: number;
  resourcesBytes: number;
  settingsBytes: number;
}

export interface MemoryStats {
  rssBytes: number;
  virtualBytes: number;
}

export interface CleanCacheResult {
  removedFiles: number;
  removedBytes: number;
  storageUsage: StorageUsage;
}

export interface StorageLocation {
  currentPath: string;
  defaultPath: string;
  isCustom: boolean;
}

export interface ChangeStorageLocationResult {
  location: StorageLocation;
  storageUsage: StorageUsage;
}

export type PreferenceDirectoryTarget = "data" | "logs";
export type BackupExportMode = "encrypted" | "plain";
export type BackupContainerMode = "encrypted" | "plain";
export type BackupReceiveSource = "dragDrop" | "openFile";
export type BackupImportStrategy = "merge" | "overwrite";

export interface ExportHistoryBackupOptions {
  mode: BackupExportMode;
  password?: string;
}

export interface ExportHistoryBackupResult {
  path: string;
  totalBytes: number;
  itemCount: number;
  textCount: number;
  imageCount: number;
  filesCount: number;
  resourceBytes: number;
  exportedAt: string;
  mode: BackupExportMode;
}

export interface ExportItemsBackupResult {
  path: string;
  totalBytes: number;
  itemCount: number;
  textCount: number;
  imageCount: number;
  filesCount: number;
  resourceBytes: number;
  exportedAt: string;
  mode: BackupExportMode;
}

export interface InspectHistoryBackupInput {
  path: string;
  source?: BackupReceiveSource;
}

export interface ImportHistoryBackupInput {
  path: string;
  password?: string;
}

export interface ImportHistoryBackupOptions {
  strategy: BackupImportStrategy;
}

export interface ImportHistoryBackupResult {
  strategy: BackupImportStrategy;
  importedItems: number;
  skippedItems: number;
  importedResources: number;
  importedSettings: boolean;
  requiresRestart: boolean;
}

export interface BackupReceivedPayload {
  path: string;
  source: BackupReceiveSource;
  mode: BackupContainerMode;
}

export type WindowLifecyclePhase =
  | "notCreated"
  | "created"
  | "ready"
  | "visible"
  | "hiddenWarm"
  | "dormant"
  | "destroyPending"
  | "destroyed";

export interface WindowLifecycleSnapshot {
  label: string;
  phase: WindowLifecyclePhase;
  generation: number;
  visible: boolean;
  retainPolicy: "permanent" | "destroyWhenIdle";
  dirtyOwnerCount: number;
  keepaliveCount: number;
  hiddenForMs: number | null;
  lastActiveAgoMs: number;
}

export interface UpdateMetadata {
  currentVersion: string;
  version: string;
  date: string | null;
  body: string | null;
  target: string;
  downloadUrl: string;
  downloaded: boolean;
}

export interface AppUpdateStatus {
  currentVersion: string;
  update: UpdateMetadata | null;
}

export interface AdminLaunchStatus {
  configured: boolean;
  runningAsAdmin: boolean;
  taskReady: boolean;
}

export interface UpdateDownloadProgress {
  downloaded: number;
  total: number | null;
  progress: number | null;
}

export interface OnboardingLegacyDataDetection {
  found: boolean;
  favoriteItemCount: number;
  importableDatabase: string | null;
  importableItemCount: number;
  normalItemCount: number;
  databaseFiles: string[];
  checkedAt: string;
  path: string | null;
  scanMessages: string[];
}

export type LegacyImportSelection = "normal" | "favorite";

export interface OnboardingLegacyImportResult {
  importedAt: string;
  importedFavorite: number;
  importedNormal: number;
  imported: number;
  selectedTypes: LegacyImportSelection[];
  skipped: number;
}

/**
 * 把任意 invoke reject 的值归一化成前端可展示的 `AppError`。
 */
const toAppError = (error: unknown): AppError => {
  if (
    typeof error === "object" &&
    error !== null &&
    "kind" in error &&
    "message" in error
  ) {
    return error as AppError;
  }

  return { kind: "Unknown", message: String(error) };
};

/**
 * invoke 的通用包装：失败 → log + toast + rethrow。
 * `label` 用于 toast 文案（"xxx 失败：message"）。
 */
const call = async <T>(
  command: string,
  labelKey: string,
  args?: Record<string, unknown>,
): Promise<T> => {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const appError = toAppError(error);

    log.error(`invoke ${command} failed`, appError);
    getMessageApi().error(
      i18n.t("commands:error", {
        label: i18n.t(labelKey),
        message: appError.message,
      }),
    );

    throw appError;
  }
};

/**
 * 拉取设置首屏快照；后续刷新走 `settings://updated` 事件。
 */
export const getSettings = () => {
  return call<Settings>(
    TAURI_COMMAND.GET_SETTINGS,
    "commands:labels.loadSettings",
  );
};

/**
 * 读取 Windows 管理员启动状态：配置、当前进程权限和计划任务准备状态。
 */
export const getRunAsAdminStatus = () => {
  return call<AdminLaunchStatus>(
    TAURI_COMMAND.GET_RUN_AS_ADMIN_STATUS,
    "commands:labels.loadRunAsAdminStatus",
  );
};

/**
 * 保存是否以管理员权限启动的持久意图。
 */
export const setRunAsAdmin = (enabled: boolean) => {
  return call<Settings>(
    TAURI_COMMAND.SET_RUN_AS_ADMIN,
    "commands:labels.setRunAsAdmin",
    { enabled },
  );
};

/**
 * 拉起一个已提权的新进程；成功后 Rust 会退出当前进程。
 */
export const restartAsAdmin = () => {
  return call<void>(
    TAURI_COMMAND.RESTART_AS_ADMIN,
    "commands:labels.restartAsAdmin",
  );
};

/**
 * 创建并显示首次启动引导窗口。
 */
export const openOnboarding = () => {
  return call<void>(
    TAURI_COMMAND.OPEN_ONBOARDING,
    "commands:labels.openOnboarding",
  );
};

/**
 * 保存引导当前步骤，供中途关闭后恢复。
 */
export const setOnboardingStep = (step: number) => {
  return call<Settings>(
    TAURI_COMMAND.SET_ONBOARDING_STEP,
    "commands:labels.saveOnboarding",
    { step },
  );
};

/**
 * 标记引导完成并打开剪贴板窗口。
 */
export const finishOnboarding = () => {
  return call<Settings>(
    TAURI_COMMAND.FINISH_ONBOARDING,
    "commands:labels.finishOnboarding",
  );
};

/**
 * 只读检测旧版 EcoPaste 数据目录，不执行导入。
 */
export const detectLegacyData = () => {
  return call<OnboardingLegacyDataDetection>(
    TAURI_COMMAND.DETECT_LEGACY_DATA,
    "commands:labels.detectLegacyData",
  );
};

/**
 * 按用户选择导入旧版普通条目和/或收藏条目。
 */
export const importLegacyData = async (types: LegacyImportSelection[]) => {
  const result = await call<OnboardingLegacyImportResult>(
    TAURI_COMMAND.IMPORT_LEGACY_DATA,
    "commands:labels.importLegacyData",
    { types },
  );

  getMessageApi().success(
    i18n.t("commands:messages.legacyDataImported", {
      imported: result.imported,
      skipped: result.skipped,
    }),
  );

  return result;
};

/**
 * 暂停全局快捷键注册；录入快捷键期间避免旧绑定被直接触发。
 */
export const suspendGlobalShortcuts = () => {
  return call<void>(
    TAURI_COMMAND.SUSPEND_GLOBAL_SHORTCUTS,
    "commands:labels.suspendGlobalShortcuts",
  );
};

/**
 * 按 Rust 当前设置恢复全局快捷键注册；录入完成、取消或失焦后调用。
 */
export const resumeGlobalShortcuts = () => {
  return call<void>(
    TAURI_COMMAND.RESUME_GLOBAL_SHORTCUTS,
    "commands:labels.resumeGlobalShortcuts",
  );
};

/**
 * 提交设置补丁；Rust 落盘后广播 `settings://updated` 由各窗口回灌镜像。
 */
export const updateSettings = (patch: SettingsPatch) => {
  return call<Settings>(
    TAURI_COMMAND.UPDATE_SETTINGS,
    "commands:labels.saveSettings",
    { patch },
  );
};

/**
 * 恢复所有偏好默认值；历史记录和资源文件不受影响。
 */
export const resetSettings = async () => {
  const settings = await call<Settings>(
    TAURI_COMMAND.RESET_SETTINGS,
    "commands:labels.resetSettings",
  );

  getMessageApi().success(i18n.t("commands:messages.settingsReset"));

  return settings;
};

/**
 * 打开独立软件更新窗口。
 */
export const openUpdateWindow = () => {
  return call<void>(
    TAURI_COMMAND.OPEN_UPDATE_WINDOW,
    "commands:labels.openUpdateWindow",
  );
};

/**
 * 读取当前更新状态；不触发网络请求。
 */
export const getUpdateStatus = () => {
  return call<AppUpdateStatus>(
    TAURI_COMMAND.GET_UPDATE_STATUS,
    "commands:labels.loadUpdateStatus",
  );
};

/**
 * 手动检查更新。
 */
export const checkForUpdates = () => {
  return call<AppUpdateStatus>(
    TAURI_COMMAND.CHECK_FOR_UPDATES,
    "commands:labels.checkForUpdates",
  );
};

/**
 * 下载并校验当前更新包，进度通过 `update://progress` 推送。
 */
export const downloadUpdate = (version: string) => {
  return call<UpdateMetadata>(
    TAURI_COMMAND.DOWNLOAD_UPDATE,
    "commands:labels.downloadUpdate",
    { version },
  );
};

/**
 * 安装已下载更新。Tauri updater 会按平台重启/退出当前应用。
 */
export const installUpdate = (version: string) => {
  return call<void>(
    TAURI_COMMAND.INSTALL_UPDATE,
    "commands:labels.installUpdate",
    { version },
  );
};

/**
 * 跳过当前发现的版本。
 */
export const skipUpdateVersion = (version: string) => {
  return call<AppUpdateStatus>(
    TAURI_COMMAND.SKIP_UPDATE_VERSION,
    "commands:labels.skipUpdateVersion",
    { version },
  );
};

/**
 * 统计本地数据库、资源缓存与设置文件的占用。
 */
export const getStorageUsage = () => {
  return call<StorageUsage>(
    TAURI_COMMAND.GET_STORAGE_USAGE,
    "commands:labels.loadStorageUsage",
  );
};

/**
 * 读取当前真实数据目录位置。
 */
export const getStorageLocation = () => {
  return call<StorageLocation>(
    TAURI_COMMAND.GET_STORAGE_LOCATION,
    "commands:labels.loadStorageLocation",
  );
};

/**
 * 读取当前 Rust 进程的内存占用，用于诊断面板。
 */
export const getProcessMemoryStats = () => {
  return call<MemoryStats>(
    TAURI_COMMAND.GET_PROCESS_MEMORY_STATS,
    "commands:labels.loadProcessMemoryStats",
  );
};

/**
 * 将数据迁移到用户选择的父目录下，并热切换运行时数据根。
 */
export const changeStorageLocation = async (targetParentDir: string) => {
  const result = await call<ChangeStorageLocationResult>(
    TAURI_COMMAND.CHANGE_STORAGE_LOCATION,
    "commands:labels.changeStorageLocation",
    { targetParentDir },
  );

  getMessageApi().success(i18n.t("commands:messages.storageLocationChanged"));

  return result;
};

/**
 * 将数据迁回默认目录，并热切换运行时数据根。
 */
export const resetStorageLocation = async () => {
  const result = await call<ChangeStorageLocationResult>(
    TAURI_COMMAND.RESET_STORAGE_LOCATION,
    "commands:labels.resetStorageLocation",
  );

  getMessageApi().success(i18n.t("commands:messages.storageLocationReset"));

  return result;
};

/**
 * 清理不再被历史记录或资源索引引用的本地资源缓存。
 */
export const cleanResourceCache = async () => {
  const result = await call<CleanCacheResult>(
    TAURI_COMMAND.CLEAN_RESOURCE_CACHE,
    "commands:labels.cleanCache",
  );

  const messageKey =
    result.removedFiles === 0 && result.removedBytes === 0
      ? "commands:messages.cacheAlreadyClean"
      : "commands:messages.cacheCleaned";

  getMessageApi().success(
    i18n.t(messageKey, {
      count: result.removedFiles,
      size: formatCommandBytes(result.removedBytes),
    }),
  );

  return result;
};

/**
 * 压缩数据库：checkpoint 收缩 WAL + VACUUM 回收 DELETE 留下的文件空洞。
 * 返回压缩后的数据库字节数供前端刷新存储占用。
 */
export const compactDatabase = async () => {
  const databaseBytes = await call<number>(
    TAURI_COMMAND.COMPACT_DATABASE,
    "commands:labels.compactDatabase",
  );

  getMessageApi().success(
    i18n.t("commands:messages.databaseCompacted", {
      size: formatCommandBytes(databaseBytes),
    }),
  );

  return databaseBytes;
};

/**
 * 打开偏好页固定本地目录：数据目录或日志目录。
 */
export const openPreferenceDirectory = (target: PreferenceDirectoryTarget) => {
  return call<void>(
    TAURI_COMMAND.OPEN_PREFERENCE_DIRECTORY,
    "commands:labels.openDirectory",
    {
      target,
    },
  );
};

/**
 * 导出历史数据库、资源和设置为 `.ecopastebak` 备份包。
 */
export const exportHistoryBackup = async (
  targetPath: string,
  options: ExportHistoryBackupOptions,
) => {
  const result = await call<ExportHistoryBackupResult>(
    TAURI_COMMAND.EXPORT_HISTORY_BACKUP,
    "commands:labels.exportBackup",
    {
      options,
      targetPath,
    },
  );

  getMessageApi().success(
    i18n.t("commands:messages.backupExported", {
      count: result.itemCount,
      size: formatCommandBytes(result.totalBytes),
    }),
  );

  return result;
};

/**
 * 把多选工具条所选条目导出为 `.ecopastebak`（格式与全量备份一致，导入端无差别）。
 * 成功后 toast 实际导出条数与备份大小；取消保存路径时上层不发调用。
 */
export const exportItemsBackup = async (
  ids: string[],
  targetPath: string,
  options: ExportHistoryBackupOptions,
) => {
  const result = await call<ExportItemsBackupResult>(
    TAURI_COMMAND.EXPORT_ITEMS_BACKUP,
    "commands:labels.exportBackup",
    { ids, options, targetPath },
  );

  getMessageApi().success(
    i18n.t("commands:messages.backupExported", {
      count: result.itemCount,
      size: formatCommandBytes(result.totalBytes),
    }),
  );

  return result;
};

/**
 * 识别 `.ecopastebak` 文件并广播给偏好页导入接收壳。
 */
export const inspectHistoryBackup = (input: InspectHistoryBackupInput) => {
  return call<BackupContainerMode>(
    TAURI_COMMAND.INSPECT_HISTORY_BACKUP,
    "commands:labels.inspectBackup",
    { input },
  );
};

/**
 * 取走偏好窗口重建前 Rust 暂存的备份接收事件，供重建后首屏补发。
 * 偏好窗口空闲销毁后再触发备份打开时，事件无法 push 给尚未挂载的前端，改由此主动拉取。
 * 失败不弹 toast：属内部补发信号，失败只记日志。
 */
export const takePendingBackup = async () => {
  try {
    return await invoke<BackupReceivedPayload | null>(
      TAURI_COMMAND.TAKE_PENDING_BACKUP,
    );
  } catch (error) {
    log.error("take pending backup failed", toAppError(error));

    return null;
  }
};

/**
 * 打开偏好窗口并定位到指定设置项。偏好窗口空闲销毁后也能在重建后正确跳转，
 * 替代前端 `showWindow` 后直接 `emitTo`（重建异步会丢事件）。
 */
export const openPreferenceWithHighlight = (settingId: string) => {
  return call<void>(
    TAURI_COMMAND.OPEN_PREFERENCE_WITH_HIGHLIGHT,
    "commands:labels.openWindow",
    { settingId },
  );
};

/**
 * 取走偏好窗口重建前 Rust 暂存的高亮目标设置项，供重建后首屏补发跳转。
 * 失败不弹 toast：属内部补发信号，失败只记日志。
 */
export const takePendingPreferenceHighlight = async () => {
  try {
    return await invoke<string | null>(
      TAURI_COMMAND.TAKE_PENDING_PREFERENCE_HIGHLIGHT,
    );
  } catch (error) {
    log.error("take pending preference highlight failed", toAppError(error));

    return null;
  }
};

/**
 * 从 `.ecopastebak` 备份包导入历史和/或设置。
 */
export const importHistoryBackup = async (
  input: ImportHistoryBackupInput,
  options: ImportHistoryBackupOptions,
) => {
  const result = await call<ImportHistoryBackupResult>(
    TAURI_COMMAND.IMPORT_HISTORY_BACKUP,
    "commands:labels.importBackup",
    {
      input,
      options,
    },
  );

  getMessageApi().success(
    i18n.t(
      result.strategy === "overwrite"
        ? "commands:messages.backupOverwriteImported"
        : "commands:messages.backupImported",
      {
        imported: result.importedItems,
        skipped: result.skippedItems,
      },
    ),
  );

  return result;
};

/**
 * 命令层 toast 使用的轻量字节格式化，避免偏好页工具反向依赖命令入口。
 */
const formatCommandBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;

  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value.toFixed(value >= 10 ? 1 : 2)} ${units[unitIndex]}`;
};

/**
 * 使用系统默认浏览器打开经过 Rust 侧校验的外部网页。
 */
export const openExternalUrl = (url: string) => {
  return call<void>(
    TAURI_COMMAND.OPEN_EXTERNAL_URL,
    "commands:labels.openLink",
    { url },
  );
};

/**
 * 查询系统自启动真实状态（auto-launch 后端）。
 */
export const getAutostart = () => {
  return call<boolean>(
    TAURI_COMMAND.GET_AUTOSTART,
    "commands:labels.loadAutostart",
  );
};

/**
 * 设置系统自启动真实状态；偏好页需与 `general.autoStart` 一起更新。
 */
export const setAutostart = (enabled: boolean) => {
  return call<void>(
    TAURI_COMMAND.SET_AUTOSTART,
    "commands:labels.setAutostart",
    {
      enabled,
    },
  );
};

/**
 * 列出可过滤应用：DB 已知应用加上当前运行中应用。
 */
export const listAllApps = () => {
  return call<ClipboardApp[]>(
    TAURI_COMMAND.LIST_ALL_APPS,
    "commands:labels.loadApps",
  );
};

/**
 * 手动添加一个来源应用，并返回写入后的应用信息。
 */
export const addClipboardAppFromPath = (path: string) => {
  return call<ClipboardApp>(
    TAURI_COMMAND.ADD_CLIPBOARD_APP_FROM_PATH,
    "commands:labels.addApp",
    { path },
  );
};

/**
 * 删除没有被历史记录引用的来源应用，并返回实际删除的应用 id。
 */
export const deleteUnreferencedClipboardApps = (ids: string[]) => {
  return call<string[]>(
    TAURI_COMMAND.DELETE_UNREFERENCED_CLIPBOARD_APPS,
    "commands:labels.deleteApps",
    { ids },
  );
};

/**
 * 列表查询；返回顶页项 + 总数 + `hasMore`，供列表分页与 Footer 共用。
 */
export const listClipboardItems = (query: ClipboardItemQuery) => {
  return call<ClipboardItemPage>(
    TAURI_COMMAND.LIST_CLIPBOARD_ITEMS,
    "commands:labels.loadClipboardList",
    { query },
  );
};

/**
 * 按 id 拉取单条「列表视图」条目（text 类型 content 已裁剪），供
 * `clipboard://updated` 事件增量刷新新条目——替代整页重拉。
 * 条目不存在（刚被清理/删除）返回 `null`，调用方降级到 reload。
 */
export const getClipboardItem = (id: string) => {
  return call<ClipboardItem | null>(
    TAURI_COMMAND.GET_CLIPBOARD_ITEM,
    "commands:labels.loadClipboardList",
    {
      id,
    },
  );
};

/**
 * 列出自定义剪贴板分组；隐藏态由调用方按场景决定是否过滤。
 */
export const listClipboardGroups = () => {
  return call<ClipboardGroupRecord[]>(
    TAURI_COMMAND.LIST_CLIPBOARD_GROUPS,
    "commands:labels.loadClipboardGroups",
  );
};

/**
 * 新建自定义剪贴板分组。
 */
export const createClipboardGroup = async (input: ClipboardGroupInput) => {
  const group = await call<ClipboardGroupRecord>(
    TAURI_COMMAND.CREATE_CLIPBOARD_GROUP,
    "commands:labels.saveClipboardGroup",
    { input },
  );

  getMessageApi().success(i18n.t("commands:messages.clipboardGroupSaved"));

  return group;
};

/**
 * 更新自定义剪贴板分组。
 */
export const updateClipboardGroup = async (
  id: string,
  input: ClipboardGroupInput,
) => {
  await call<void>(
    TAURI_COMMAND.UPDATE_CLIPBOARD_GROUP,
    "commands:labels.saveClipboardGroup",
    { id, input },
  );

  getMessageApi().success(i18n.t("commands:messages.clipboardGroupSaved"));
};

/**
 * 保存自定义剪贴板分组的排序和主界面显隐状态。
 */
export const updateClipboardGroupsLayout = async (
  order: string[],
  visibleIds: string[],
) => {
  await call<void>(
    TAURI_COMMAND.UPDATE_CLIPBOARD_GROUPS_LAYOUT,
    "commands:labels.saveClipboardGroupsLayout",
    { input: { order, visibleIds } },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardGroupsLayoutSaved"),
  );
};

/**
 * 删除自定义剪贴板分组。
 */
export const deleteClipboardGroup = async (id: string) => {
  await call<void>(
    TAURI_COMMAND.DELETE_CLIPBOARD_GROUP,
    "commands:labels.deleteClipboardGroup",
    { id },
  );

  getMessageApi().success(i18n.t("commands:messages.clipboardGroupDeleted"));
};

/**
 * 将单条剪贴板记录移动到指定自定义分组。
 */
export const updateClipboardItemGroup = async (id: string, groupId: string) => {
  await call<void>(
    TAURI_COMMAND.UPDATE_CLIPBOARD_ITEM_GROUP,
    "commands:labels.moveToGroup",
    { groupId, id },
  );

  getMessageApi().success(i18n.t("commands:messages.itemMovedToGroup"));
};

/**
 * 读取 Tauri dialog 选中的 SVG 文件内容。
 */
export const importClipboardGroupSvg = (path: string) => {
  return call<string>(
    TAURI_COMMAND.IMPORT_CLIPBOARD_GROUP_SVG,
    "commands:labels.importClipboardGroupSvg",
    { path },
  );
};

/**
 * 打开条目 URL：`mailto = true` 时 Rust 侧自动裹 `mailto:`。
 * 用于右键菜单「打开链接 / 发送邮件」。
 */
export const openClipboardItemLink = (id: string, mailto: boolean) => {
  return call<void>(
    TAURI_COMMAND.OPEN_CLIPBOARD_ITEM_LINK,
    "commands:labels.openLink",
    {
      id,
      mailto,
    },
  );
};

/**
 * 在系统文件管理器中定位条目对应文件；Rust 侧自动按 kind 提路径（files 取首个，text 取 content）。
 */
export const revealClipboardItem = (id: string) => {
  return call<void>(
    TAURI_COMMAND.REVEAL_CLIPBOARD_ITEM,
    "commands:labels.reveal",
    { id },
  );
};

/**
 * 将图片历史记录另存为本地 PNG 文件；用户取消保存时返回 `null` 且不提示成功。
 */
export const saveClipboardImageToFile = async (id: string) => {
  const path = await call<string | null>(
    TAURI_COMMAND.SAVE_CLIPBOARD_IMAGE_TO_FILE,
    "commands:labels.saveImage",
    { id },
  );

  if (path !== null) {
    getMessageApi().success(i18n.t("commands:messages.imageSaved"));
  }

  return path;
};

/**
 * 写回剪贴板（不模拟粘贴）：右键菜单「复制」走此命令。
 * `plain` 为显式纯文本动作；默认复制格式由 Rust 按设置与记录类型决定。
 * 成功后统一 toast「已复制」，调用方无需再处理。
 */
export const writeToClipboard = async (
  id: string,
  plain: boolean,
  options: { silent?: boolean } = {},
) => {
  await call<void>(TAURI_COMMAND.WRITE_TO_CLIPBOARD, "commands:labels.copy", {
    id,
    plain,
  });

  if (options.silent) return;

  getMessageApi().success(i18n.t("commands:messages.copied"));
};

/**
 * 把任意纯文本写入剪贴板（预览面板「复制选中片段」）。
 *
 * 与 `writeToClipboard` 的区别：不按记录 id 写回，也不登记回环抑制——
 * 期望 OS 监听管线把这段文本按既有去重 / 搜索语义收成一条新记录。
 *
 * `silent` 用于预览窗内调用：那边已有按钮自身的 ✓ 反馈，再弹一次 antd toast
 * 会在两个窗口各飘一条，视觉上重复。
 */
export const writeTextToClipboard = async (
  text: string,
  options: { silent?: boolean } = {},
) => {
  await call<void>(
    TAURI_COMMAND.WRITE_TEXT_TO_CLIPBOARD,
    "commands:labels.copy",
    {
      text,
    },
  );

  if (options.silent) return;

  getMessageApi().success(i18n.t("commands:messages.copied"));
};

/**
 * 「写回剪贴板 + 隐藏剪贴板窗口 + 模拟系统粘贴」的组合命令。
 * `plain` 为显式纯文本 / 路径粘贴动作；默认粘贴格式由 Rust 按设置与记录类型决定。
 * `transform` 为文本清理变换（仅文本条目）：变换后恒走纯文本写回且不进历史；
 * 不传时走既有写回语义。回车 / 数字快捷键 / 右键菜单全部走这里。
 */
export const pasteClipboardItem = (
  id: string,
  plain: boolean,
  transform?: PasteTransform | null,
) => {
  return call<void>(
    TAURI_COMMAND.PASTE_CLIPBOARD_ITEM,
    "commands:labels.paste",
    { id, plain, transform: transform ?? null },
  );
};

/**
 * 多选合并粘贴：按 `ids` 传入序拼接纯文本后一次写回 + 模拟粘贴。
 * 调用方需先按列表显示序（顶→底）排好 `ids`；`separator` 取自
 * `clipboard.content.mergePasteSeparator` 设置镜像。合成串不进历史，
 * 逐条 `use_count` 由 Rust 按 `updateOnReuse` 开关累加。
 */
export const pasteClipboardItems = (
  ids: string[],
  separator: MergePasteSeparator,
  plain: boolean,
) => {
  return call<void>(
    TAURI_COMMAND.PASTE_CLIPBOARD_ITEMS,
    "commands:labels.paste",
    { ids, plain, separator },
  );
};

/**
 * 启动一次 OS 级 drag-out：把条目拖出剪贴板窗口到外部应用。
 *
 * - Files / Image：拖出为文件，预览用 OS 原生图标。
 * - Text（含 HTML / RTF 富格式）：接收方按偏好选格式；Rust 端用文本首几行
 *   现场渲染的 PNG 作预览，缺失则退回来源 app 图标。
 *
 * macOS 立即返回（drop 由 OS 异步处理）；Windows 会 await 至 drop 完成。
 * 失败已在 `call` 内统一 toast，调用方一般不需要再处理。
 */
export const startDragClipboardItem = (id: string) => {
  return call<void>(
    TAURI_COMMAND.START_DRAG_CLIPBOARD_ITEM,
    "commands:labels.drag",
    { id },
  );
};

/**
 * 翻转收藏态；`favorite` 表示本次期望的新状态（用于 toast 文案）。
 * Rust 返回翻转后的真实状态，调用方据此同步 UI。
 * 成功后统一 toast「已收藏 / 已取消收藏」，失败也按意图分开「收藏失败 / 取消收藏失败」。
 */
export const toggleClipboardItemFavorite = async (
  id: string,
  favorite: boolean,
) => {
  const next = await call<boolean>(
    TAURI_COMMAND.TOGGLE_CLIPBOARD_ITEM_FAVORITE,
    favorite
      ? "commands:labels.toggleFavorite"
      : "commands:labels.cancelFavorite",
    { id },
  );

  getMessageApi().success(
    i18n.t(
      next
        ? "commands:messages.favoriteAdded"
        : "commands:messages.favoriteRemoved",
    ),
  );

  return next;
};

/**
 * 翻转置顶态；`pinned` 表示本次期望的新状态，用于失败 toast 文案。
 * Rust 返回翻转后的真实状态，调用方据此同步 UI 和列表排序。
 */
export const toggleClipboardItemPinned = async (
  id: string,
  pinned: boolean,
) => {
  const next = await call<boolean>(
    TAURI_COMMAND.TOGGLE_CLIPBOARD_ITEM_PINNED,
    pinned ? "commands:labels.pinItem" : "commands:labels.unpinItem",
    { id },
  );

  getMessageApi().success(
    i18n.t(
      next ? "commands:messages.itemPinned" : "commands:messages.itemUnpinned",
    ),
  );

  return next;
};

/**
 * 删除条目；命令**不**广播 `clipboard://updated`，调用方需根据返回值本地移除该项。
 * 普通条目、收藏条目与置顶条目分别读取对应保护 / 确认开关。
 * 成功后统一 toast「已删除」。
 */
export const deleteClipboardItem = async (
  id: string,
  isFavorite: boolean,
  isPinned: boolean,
): Promise<boolean> => {
  const contentSettings = settingsState.clipboard?.content;

  if (isFavorite && !(contentSettings?.deleteFavoriteItems ?? false)) {
    return false;
  }

  if (isPinned && !(contentSettings?.deletePinnedItems ?? false)) {
    return false;
  }

  const needConfirm =
    (isFavorite && (contentSettings?.deleteFavoriteConfirm ?? true)) ||
    (isPinned && (contentSettings?.deletePinnedConfirm ?? true)) ||
    (!isFavorite && !isPinned && (contentSettings?.deleteConfirm ?? true));

  if (needConfirm) {
    const ok = await new Promise<boolean>((resolve) => {
      getModalApi().confirm({
        cancelText: i18n.t("common:actions.cancel"),
        centered: true,
        content: i18n.t("commands:deleteConfirm.content"),
        okButtonProps: { danger: true },
        okText: i18n.t("common:actions.delete"),
        onCancel: () => resolve(false),
        onOk: () => resolve(true),
        title: i18n.t("commands:deleteConfirm.title"),
      });
    });

    if (!ok) return false;
  }

  await call<void>(
    TAURI_COMMAND.DELETE_CLIPBOARD_ITEM,
    "commands:labels.delete",
    { id },
  );

  getMessageApi().success(i18n.t("commands:messages.deleted"));

  return true;
};

/**
 * 批量删除多选条目。与单条删除同套保护 / 确认开关：
 * 含受保护条目且未开启对应「允许删除」设置时整体放弃（与单条行为一致，不做部分删除）；
 * 确认弹层展示选中数量。成功后 toast 删除计数。
 */
export const deleteClipboardItems = async (
  items: Array<{ id: string; isFavorite: boolean; isPinned: boolean }>,
): Promise<boolean> => {
  if (items.length === 0) return false;

  const contentSettings = settingsState.clipboard?.content;
  const hasFavorite = items.some((item) => item.isFavorite);
  const hasPinned = items.some((item) => item.isPinned);

  if (hasFavorite && !(contentSettings?.deleteFavoriteItems ?? false)) {
    return false;
  }

  if (hasPinned && !(contentSettings?.deletePinnedItems ?? false)) {
    return false;
  }

  const needConfirm =
    (hasFavorite && (contentSettings?.deleteFavoriteConfirm ?? true)) ||
    (hasPinned && (contentSettings?.deletePinnedConfirm ?? true)) ||
    (!hasFavorite && !hasPinned && (contentSettings?.deleteConfirm ?? true));

  if (needConfirm) {
    const ok = await new Promise<boolean>((resolve) => {
      getModalApi().confirm({
        cancelText: i18n.t("common:actions.cancel"),
        centered: true,
        content: i18n.t("commands:deleteSelectedConfirm.content", {
          count: items.length,
        }),
        okButtonProps: { danger: true },
        okText: i18n.t("common:actions.delete"),
        onCancel: () => resolve(false),
        onOk: () => resolve(true),
        title: i18n.t("commands:deleteSelectedConfirm.title"),
      });
    });

    if (!ok) return false;
  }

  const removed = await call<number>(
    TAURI_COMMAND.DELETE_CLIPBOARD_ITEMS,
    "commands:labels.delete",
    {
      deleteFavorites: true,
      deletePinned: true,
      ids: items.map((item) => item.id),
    },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardItemsDeleted", { count: removed }),
  );

  return true;
};

/**
 * 清空剪贴板历史；默认保留收藏和置顶，确认选项决定是否连带删除受保护记录。
 */
export const clearClipboardItems = async (): Promise<boolean> => {
  const options = await confirmClearClipboardItems();

  if (!options) return false;

  const removed = await call<number>(
    TAURI_COMMAND.CLEAR_CLIPBOARD_ITEMS,
    "commands:labels.clearClipboardItems",
    {
      deleteFavorites: options.deleteFavorites,
      deletePinned: options.deletePinned,
    },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardItemsCleared", { count: removed }),
  );

  return true;
};

/**
 * 批量设置收藏态（多选工具条）。Rust 分块 UPDATE 后广播刷新，返回受影响行数。
 */
export const setClipboardItemsFavorite = async (
  ids: string[],
  favorite: boolean,
) => {
  if (ids.length === 0) return 0;

  const affected = await call<number>(
    TAURI_COMMAND.SET_CLIPBOARD_ITEMS_FAVORITE,
    "commands:labels.batchUpdateItems",
    { favorite, ids },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardItemsUpdated", { count: affected }),
  );

  return affected;
};

/**
 * 批量设置置顶态（多选工具条）。置顶影响排序，事件驱动刷新当前页。
 */
export const setClipboardItemsPinned = async (
  ids: string[],
  pinned: boolean,
) => {
  if (ids.length === 0) return 0;

  const affected = await call<number>(
    TAURI_COMMAND.SET_CLIPBOARD_ITEMS_PINNED,
    "commands:labels.batchUpdateItems",
    { ids, pinned },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardItemsUpdated", { count: affected }),
  );

  return affected;
};

/**
 * 批量移动条目到分组；`groupId` 传 `null` 移出分组。
 */
export const moveClipboardItemsToGroup = async (
  ids: string[],
  groupId: string | null,
) => {
  if (ids.length === 0) return 0;

  const affected = await call<number>(
    TAURI_COMMAND.MOVE_CLIPBOARD_ITEMS_TO_GROUP,
    "commands:labels.batchUpdateItems",
    { groupId, ids },
  );

  getMessageApi().success(
    i18n.t("commands:messages.clipboardItemsUpdated", { count: affected }),
  );

  return affected;
};

/**
 * 更新备注；Rust 统一 trim + 空串归一为 `null`，返回归一化后的 `note` 与 `autoFavorited`。
 * 调用方用返回的 `note` 回填本地镜像，避免「输入纯空白时镜像非空但 DB 为 NULL」的漂移。
 * 成功后统一 toast：触发 auto-favorite 时「已保存并收藏」，否则「已保存」。
 */
export const updateClipboardItemNote = async (
  id: string,
  note: string | null,
) => {
  const result = await call<UpdateNoteResult>(
    TAURI_COMMAND.UPDATE_CLIPBOARD_ITEM_NOTE,
    "commands:labels.saveNote",
    { id, note },
  );

  getMessageApi().success(
    i18n.t(
      result.autoFavorited
        ? "commands:messages.noteSavedAndFavorited"
        : "commands:messages.noteSaved",
    ),
  );

  return result;
};

/**
 * 读取文本条目的编辑源文本（Ditto 式 edit entry 弹窗回显）。
 * 富文本条目返回纯文本表示，其余文本条目返回 `content` 原文；
 * 非文本条目或记录不存在返回 `null`，调用方据此不打开编辑弹窗。
 */
export const getClipboardItemEditText = (id: string) => {
  return call<string | null>(
    TAURI_COMMAND.GET_CLIPBOARD_ITEM_EDIT_TEXT,
    "commands:labels.getEditText",
    { id },
  );
};

/**
 * 就地更新文本条目内容；Rust 侧单条 UPDATE 重写派生字段（hash / 搜索文本 /
 * 摘要 / 子类型 / 大小），不动最近使用时间与收藏、置顶、备注、分组等元数据，
 * FTS 由触发器自动同步。返回 enrich 后的完整列表条目，调用方直接整体回填本地镜像。
 */
export const updateClipboardItemText = async (id: string, text: string) => {
  const updated = await call<ClipboardItem>(
    TAURI_COMMAND.UPDATE_CLIPBOARD_ITEM_TEXT,
    "commands:labels.saveText",
    { content: text, id },
  );

  getMessageApi().success(i18n.t("commands:messages.textSaved"));

  return updated;
};

/**
 * 按窗口 label 显示窗口（偏好设置窗口、剪贴板窗口等）。
 */
export const showWindow = (label: string) => {
  return call<void>(TAURI_COMMAND.SHOW_WINDOW, "commands:labels.openWindow", {
    label,
  });
};

/**
 * 按窗口 label 隐藏窗口（偏好窗口、剪贴板窗口等）。
 */
export const hideWindow = (label: string) => {
  return call<void>(TAURI_COMMAND.HIDE_WINDOW, "commands:labels.closeWindow", {
    label,
  });
};

/**
 * 上报当前 WebView 已完成基础初始化，由 Rust 生命周期管理器把窗口推进到 ready 阶段。
 * 失败不弹 toast：ready handshake 属内部信号，失败只记日志，不打扰用户。
 */
export const notifyWindowReady = async (label: string) => {
  try {
    await invoke<void>(TAURI_COMMAND.NOTIFY_WINDOW_READY, { label });
  } catch (error) {
    log.error("notify window ready failed", toAppError(error));
  }
};

/**
 * 标记窗口是否存在未保存草稿；任一 owner 未清除时 Rust 会延后 idle destroy。
 */
export const setWindowDirty = async (
  label: string,
  owner: string,
  dirty: boolean,
) => {
  try {
    await invoke<void>(TAURI_COMMAND.SET_WINDOW_DIRTY, {
      dirty,
      label,
      owner,
    });
  } catch (error) {
    log.error("set window dirty failed", toAppError(error));
  }
};

/**
 * 申请窗口短期保活租约；用于原生对话框或长任务进行中避免被 idle destroy。
 */
export const acquireWindowKeepalive = async (
  label: string,
  owner: string,
  reason: string,
  timeoutMs?: number,
) => {
  try {
    await invoke<void>(TAURI_COMMAND.ACQUIRE_WINDOW_KEEPALIVE, {
      label,
      owner,
      reason,
      timeoutMs,
    });
  } catch (error) {
    log.error("acquire window keepalive failed", toAppError(error));
  }
};

/**
 * 释放窗口保活租约。
 */
export const releaseWindowKeepalive = async (label: string, owner: string) => {
  try {
    await invoke<void>(TAURI_COMMAND.RELEASE_WINDOW_KEEPALIVE, {
      label,
      owner,
    });
  } catch (error) {
    log.error("release window keepalive failed", toAppError(error));
  }
};

/**
 * 读取窗口生命周期调试快照。
 */
export const getWindowLifecycleSnapshot = async () => {
  try {
    return await invoke<WindowLifecycleSnapshot[]>(
      TAURI_COMMAND.GET_WINDOW_LIFECYCLE_SNAPSHOT,
    );
  } catch (error) {
    log.error("get window lifecycle snapshot failed", toAppError(error));

    return [];
  }
};

/**
 * 显示或隐藏 macOS Dock / Windows 任务栏图标。
 */
export const showTaskbarIcon = (visible: boolean) => {
  return call<void>(
    TAURI_COMMAND.SHOW_TASKBAR_ICON,
    "commands:labels.setTaskbarIcon",
    {
      visible,
    },
  );
};

/**
 * 设置剪贴板窗口固定态：Rust 侧立即生效（影响 resign_key / 外部点击自动隐藏逻辑）。
 */
export const setClipboardWindowPinned = (pinned: boolean) => {
  return call<void>(
    TAURI_COMMAND.SET_CLIPBOARD_WINDOW_PINNED,
    "commands:labels.setClipboardWindowPinned",
    {
      pinned,
    },
  );
};

/**
 * 临时暂停剪贴板窗口自动隐藏，供系统文件选择等原生交互保持剪贴板窗口可见。
 */
export const setClipboardWindowAutoHideSuspended = (suspended: boolean) => {
  return call<void>(
    TAURI_COMMAND.SET_CLIPBOARD_WINDOW_AUTO_HIDE_SUSPENDED,
    "commands:labels.setClipboardWindowAutoHideSuspended",
    {
      suspended,
    },
  );
};

/**
 * Windows 剪贴板窗口输入编辑模式：输入控件激活期间临时可聚焦，编辑结束后恢复不可聚焦。
 */
export const setClipboardWindowEditing = async (editing: boolean) => {
  try {
    await invoke<void>(TAURI_COMMAND.SET_CLIPBOARD_WINDOW_EDITING, {
      editing,
    });
  } catch (error) {
    log.error("set clipboard window editing failed", toAppError(error));
  }
};

/**
 * 搜索框聚焦完成后回报 Rust，解锁回放被吞的可打印字符（Ditto 式随时输入即搜索）。
 */
export const searchTypingAck = async () => {
  try {
    await invoke<void>(TAURI_COMMAND.SEARCH_TYPING_ACK);
  } catch (error) {
    log.error("search typing ack failed", toAppError(error));
  }
};

/**
 * 打开或重定向剪贴板系统级预览 overlay。
 * `anchor` 是剪贴板窗口 webview client 坐标中的列表项矩形。
 */
export const showClipboardPreview = (
  itemId: string,
  anchor: PreviewAnchorRect,
) => {
  return call<ClipboardPreviewState | null>(
    TAURI_COMMAND.SHOW_CLIPBOARD_PREVIEW,
    "commands:labels.openPreview",
    { anchor, itemId },
  );
};

/**
 * 关闭剪贴板系统级预览 overlay。
 * `reason` 只用于日志：前端无法预知「哪条关闭路径」会被触发，写进 Rust 日志便于回溯。
 */
export const closeClipboardPreview = (reason: string) => {
  return call<void>(
    TAURI_COMMAND.CLOSE_CLIPBOARD_PREVIEW,
    "commands:labels.closePreview",
    { reason },
  );
};

/**
 * 预览窗口首屏补拉最近一次状态。
 */
export const getClipboardPreviewState = () => {
  return call<ClipboardPreviewState | null>(
    TAURI_COMMAND.GET_CLIPBOARD_PREVIEW_STATE,
    "commands:labels.loadPreviewState",
  );
};

/**
 * 读取预览窗口 Content Viewer 所需的归一化 payload。
 */
export const getClipboardPreviewPayload = (itemId: string) => {
  return call<ClipboardPreviewPayload | null>(
    TAURI_COMMAND.GET_CLIPBOARD_PREVIEW_PAYLOAD,
    "commands:labels.loadPreviewContent",
    { itemId },
  );
};

/**
 * 上报预览面板的实测矩形（overlay 局部逻辑坐标），供 Rust 维护鼠标命中判定。
 * 随面板动画高频调用；失败可自愈（Rust 回退到 layout 面板框），故只记日志不弹 toast。
 */
export const setClipboardPreviewPanelRect = async (
  rect: ClipboardPreviewRect,
) => {
  try {
    await invoke<void>(TAURI_COMMAND.SET_CLIPBOARD_PREVIEW_PANEL_RECT, {
      rect,
    });
  } catch (error) {
    log.error("set clipboard preview panel rect failed", toAppError(error));
  }
};

/**
 * 上报指针进入 / 离开预览面板。离开方向必须立即回报：翻转期间整个全屏 overlay
 * 都在接收鼠标事件，等 Rust 采样周期会让面板外的点击短暂失效。
 */
export const setClipboardPreviewPointer = async (inside: boolean) => {
  try {
    await invoke<void>(TAURI_COMMAND.SET_CLIPBOARD_PREVIEW_POINTER, { inside });
  } catch (error) {
    log.error("set clipboard preview pointer failed", toAppError(error));
  }
};

/**
 * 播放一次复制成功提示音，供偏好设置页试听。
 */
export const playCopySound = () => {
  return call<void>(
    TAURI_COMMAND.PLAY_COPY_SOUND,
    "commands:labels.playCopySound",
  );
};

/**
 * 在剪贴板窗口当前光标处弹出列表项右键菜单（菜单实例由 Rust 持有）。
 *
 * 点击菜单项后 Rust 会 emit `clipboard://menu-action` 携带 `{action, itemId}`，
 * 由 `List.tsx` 单点订阅后派发到既有处理逻辑（toast / 确认 modal / 本地镜像同步）。
 *
 * 把菜单生命周期搬到 Rust 是为了规避 tauri-apps/tauri#9470：前端 `Menu.new` 在
 * `popup` 后立即被 GC 会导致 Windows muda 点击崩溃/卡顿。
 */
export const popupClipboardItemMenu = (
  itemId: string,
  availableActions: ClipboardAction[],
  currentGroupId: string | null,
  isFavorite: boolean,
  isPinned: boolean,
  hasNote: boolean,
) => {
  return call<void>(
    TAURI_COMMAND.POPUP_CLIPBOARD_ITEM_MENU,
    "commands:labels.openMenu",
    {
      input: {
        availableActions,
        currentGroupId,
        hasNote,
        isFavorite,
        isPinned,
        itemId,
      },
    },
  );
};

/**
 * 读取 Windows 自定义右键菜单一级窗口的待渲染 payload。
 * 窗口被自动销毁后重建时用于首屏补拉，失败只记录日志。
 */
export const getContextMenuPayload = async () => {
  try {
    return await invoke<ContextMenuShowPayload | null>(
      TAURI_COMMAND.GET_CONTEXT_MENU_PAYLOAD,
    );
  } catch (error) {
    log.error("get context menu payload failed", toAppError(error));

    return null;
  }
};

/**
 * 读取 Windows 自定义右键菜单二级窗口的待渲染 payload。
 * 窗口被自动销毁后重建时用于首屏补拉，失败只记录日志。
 */
export const getContextSubmenuPayload = async () => {
  try {
    return await invoke<ShowContextSubmenuInput | null>(
      TAURI_COMMAND.GET_CONTEXT_SUBMENU_PAYLOAD,
    );
  } catch (error) {
    log.error("get context submenu payload failed", toAppError(error));

    return null;
  }
};

/**
 * 显示 Windows 自定义右键菜单的二级窗口。
 * 内部菜单生命周期命令失败只记日志，避免 hover 过程中打扰用户。
 */
export const showContextSubmenu = async (input: ShowContextSubmenuInput) => {
  try {
    await invoke<void>(TAURI_COMMAND.SHOW_CONTEXT_SUBMENU, { input });
  } catch (error) {
    log.error("show context submenu failed", toAppError(error));
  }
};

/**
 * 隐藏 Windows 自定义右键菜单的二级窗口。
 */
export const hideContextSubmenu = async () => {
  try {
    await invoke<void>(TAURI_COMMAND.HIDE_CONTEXT_SUBMENU);
  } catch (error) {
    log.error("hide context submenu failed", toAppError(error));
  }
};

/**
 * 隐藏 Windows 自定义右键菜单的一级和二级窗口。
 */
export const hideContextMenus = async () => {
  try {
    await invoke<void>(TAURI_COMMAND.HIDE_CONTEXT_MENUS);
  } catch (error) {
    log.error("hide context menus failed", toAppError(error));
  }
};
