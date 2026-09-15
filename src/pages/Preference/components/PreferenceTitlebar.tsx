import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { FC } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/utils/cn";
import { isMac } from "@/utils/is";

/**
 * 偏好窗口自定义标题栏：整条可拖拽，背景与侧边栏一致，不显示图标与应用名。
 *
 * - Windows：关闭原生装饰后由本组件承担拖拽 + 最小化/关闭按钮。
 * - macOS：保留原生红绿灯，仅左侧留出让位空间，不渲染任何按钮。
 * - 拖拽：`data-tauri-drag-region` 启用原生拖拽；双击标题栏切换最大化由 Tauri 内置。
 */
const PreferenceTitlebar: FC = () => {
  const { t } = useTranslation("common");

  const handleMinimize = () => {
    void getCurrentWebviewWindow().minimize();
  };

  const handleClose = () => {
    // 偏好窗口关闭 = 隐藏：Rust 侧 CloseRequested 拦截器负责保存几何并转 hide。
    void getCurrentWebviewWindow().close();
  };

  return (
    <header
      className={cn(
        "flex h-8 shrink-0 select-none items-center bg-ant-container",
        isMac && "pl-19.5",
      )}
      data-tauri-drag-region
    >
      {/* 拖拽填充区：撑满剩余宽度，维持可拖拽面积 */}
      <div className="h-full min-w-0 flex-1" data-tauri-drag-region />

      {!isMac && (
        <div className="flex h-full shrink-0">
          <button
            aria-label={t("window.minimize")}
            className={cn(
              "flex h-8 w-11.5 cursor-pointer items-center justify-center border-0 bg-transparent text-ant-text transition-colors focus-visible:ring-1 focus-visible:ring-ant-primary motion-reduce:transition-none",
              "hover:bg-ant-fill-tertiary active:bg-ant-fill-quaternary",
            )}
            onClick={handleMinimize}
            title={t("window.minimize")}
            type="button"
          >
            <i aria-hidden="true" className="i-lucide:minus text-sm" />
          </button>
          <button
            aria-label={t("window.close")}
            className={cn(
              "flex h-8 w-11.5 cursor-pointer items-center justify-center border-0 bg-transparent text-ant-text transition-colors focus-visible:ring-1 focus-visible:ring-ant-primary motion-reduce:transition-none",
              "hover:bg-ant-error hover:text-ant-white active:opacity-90",
            )}
            onClick={handleClose}
            title={t("window.close")}
            type="button"
          >
            <i aria-hidden="true" className="i-lucide:x text-sm" />
          </button>
        </div>
      )}
    </header>
  );
};

export default PreferenceTitlebar;
