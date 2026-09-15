import { useMount } from "ahooks";
import { Spin } from "antd";
import { motion } from "motion/react";
import {
  type FC,
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { useSnapshot } from "valtio";
import {
  type ClipboardPreviewState,
  getClipboardPreviewState,
  setClipboardPreviewPanelRect,
  setClipboardPreviewPointer,
} from "@/commands";
import { TAURI_EVENT } from "@/constants/events";
import { WINDOW_LABEL } from "@/constants/windows";
import { useTauriListen } from "@/hooks/useTauriListen";
import { settingsState } from "@/stores/settings";
import { cn } from "@/utils/cn";
import { log } from "@/utils/log";
import { cacheKey } from "./cache";
import { PreviewContent, PreviewHeader } from "./components/PreviewContent";
import PreviewContentTransition from "./components/PreviewContentTransition";
import SelectionCopyButton from "./components/SelectionCopyButton";
import {
  PREVIEW_CONNECTOR_VARIANTS,
  PREVIEW_PANEL_MARGIN,
  PREVIEW_PANEL_MAX_HEIGHT,
  PREVIEW_PANEL_TRANSITION,
  PREVIEW_PANEL_VARIANTS,
} from "./constants";
import { resolveConnector } from "./geometry";
import { usePreviewPayload, usePreviewRenderState } from "./hooks";
import {
  rectStyle,
  resolveDynamicPanelRect,
  resolveEffectivePanelSize,
  resolveMeasurePanelStyle,
} from "./layout";
import { hasMeasuredPanelSize, useMeasuredPanelSize } from "./measurement";
import { usePreviewMotion } from "./motion";

const EMPTY_RECT = {
  height: 1,
  left: 0,
  top: 0,
  width: 1,
};
const EMPTY_POINT = { x: 0, y: 0 };
const EMPTY_CONNECTOR = {
  control1: EMPTY_POINT,
  control2: EMPTY_POINT,
  path: "M 0 0 C 0 0 0 0 0 0",
  source: EMPTY_POINT,
  sourceDot: EMPTY_POINT,
  sourceSide: "right",
  target: EMPTY_POINT,
  targetDot: EMPTY_POINT,
  targetSide: "left",
} as const;

interface BeforeDestroyPayload {
  label: string;
}

/**
 * 系统级剪贴板预览窗口。
 * 预览窗口自身常驻透明 overlay，按 `itemId + updatedAt` 缓存最近内容并渲染基础 Content Viewer。
 */
const Preview: FC = () => {
  const [previewState, setPreviewState] =
    useState<ClipboardPreviewState | null>(null);
  const [payloadResetToken, setPayloadResetToken] = useState(0);
  const [interactive, setInteractive] = useState(false);
  const panelMeasureRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const { clipboard } = useSnapshot(settingsState);
  const redactSecrets = clipboard.sensitive.redactSecrets;
  const renderState = usePreviewRenderState(previewState);
  const { loadingItemId, payload } = usePreviewPayload(
    previewState,
    payloadResetToken,
  );
  const measuredPanelSize = useMeasuredPanelSize(panelMeasureRef);
  const active = previewState !== null;
  const visibleState = previewState ?? renderState;
  const maxPanelHeight = visibleState
    ? resolveMaxPanelHeight(visibleState.layout.overlayRect.height, interactive)
    : PREVIEW_PANEL_MAX_HEIGHT;
  const effectivePanelSize = visibleState
    ? resolveEffectivePanelSize(
        visibleState.layout,
        measuredPanelSize,
        payload,
        maxPanelHeight,
      )
    : measuredPanelSize;
  const panelRect = visibleState
    ? resolveDynamicPanelRect(
        visibleState.layout,
        effectivePanelSize,
        maxPanelHeight,
      )
    : EMPTY_RECT;
  const panelMeasureStyle = visibleState
    ? resolveMeasurePanelStyle(visibleState.layout)
    : void 0;
  const connector = visibleState
    ? resolveConnector(visibleState.layout.sourceRect, panelRect)
    : EMPTY_CONNECTOR;
  const motionLayout = usePreviewMotion(
    active,
    visibleState?.sessionId ?? null,
    hasMeasuredPanelSize(effectivePanelSize),
    panelRect,
    connector,
  );

  useMount(async () => {
    try {
      const state = await getClipboardPreviewState();
      setPreviewState(state);
    } catch (error) {
      log.error("load preview state failed", error);
    }
  });

  useTauriListen<ClipboardPreviewState | null>(
    TAURI_EVENT.PREVIEW_UPDATED,
    (event) => {
      setPreviewState(event.payload);
    },
  );

  const handleBeforeDestroy = (event: { payload: BeforeDestroyPayload }) => {
    if (event.payload.label !== WINDOW_LABEL.PREVIEW) return;

    setPreviewState(null);
    setPayloadResetToken((current) => {
      return current + 1;
    });
  };

  useTauriListen<BeforeDestroyPayload>(
    TAURI_EVENT.WINDOW_BEFORE_DESTROY,
    handleBeforeDestroy,
  );

  const handlePreviewPointer = (event: { payload: { inside: boolean } }) => {
    setInteractive(event.payload.inside);
  };

  useTauriListen<{ inside: boolean }>(
    TAURI_EVENT.PREVIEW_POINTER,
    handlePreviewPointer,
  );

  // 预览关闭后交互态不再有意义，复位等待下一次 show。
  useEffect(() => {
    if (active) return;

    setInteractive(false);
  }, [active]);

  // 面板矩形上报：Rust 只有 layout 的 480 框，鼠标命中判定要用前端算出的动态面板。
  // 矩形只在内容 / 布局 / 交互态变化时改变（动画由 Motion Value 驱动），无需节流。
  // biome-ignore lint/correctness/useExhaustiveDependencies: panelRect 每轮 render 都是新对象，按字段依赖以避免每帧重复 IPC
  useEffect(() => {
    if (!active) return;

    void setClipboardPreviewPanelRect(panelRect);
  }, [
    active,
    panelRect.height,
    panelRect.left,
    panelRect.top,
    panelRect.width,
  ]);

  if (!visibleState) {
    return <div className="fixed inset-0 overflow-hidden bg-transparent" />;
  }

  const isLoading = loadingItemId !== null;
  const payloadKey = payload ? cacheKey(payload, redactSecrets) : "empty";
  const { layout } = visibleState;
  const svgStyle = rectStyle(layout.overlayRect);

  /**
   * 面板指针进出回报：离开方向必须即时翻转穿透，不能等 Rust 的采样周期
   * （翻转期间整个全屏 overlay 都在接收鼠标事件）。
   */
  const handlePanelPointerEnter = () => {
    void setClipboardPreviewPointer(true);
  };

  const handlePanelPointerLeave = (
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    // 拖选常把指针带出面板边缘：按住左键期间不回报离开，否则面板会中途交还穿透，
    // 拖选与滚动会当场断掉。松开后由 `handlePanelPointerUp` 收口。
    if (event.buttons !== 0) return;

    void setClipboardPreviewPointer(false);
  };

  /**
   * 按键松开后按真实光标位置重判（Rust 侧重新比对命中矩形）：
   * 指针已离开面板就交还穿透，仍在面板上则保持可交互。
   */
  const handlePanelPointerUp = () => {
    void setClipboardPreviewPointer(false);
  };

  return (
    <div className="fixed inset-0 overflow-hidden bg-transparent">
      <motion.svg
        animate={active ? "open" : "closed"}
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 z-10 overflow-visible"
        initial="closed"
        role="presentation"
        style={svgStyle}
        transition={PREVIEW_PANEL_TRANSITION}
        variants={PREVIEW_CONNECTOR_VARIANTS}
        viewBox={`0 0 ${layout.overlayRect.width} ${layout.overlayRect.height}`}
      >
        <motion.path
          className="stroke-ant-primary"
          d={motionLayout.path}
          fill="none"
          strokeLinecap="round"
          strokeWidth="2"
        />
        <motion.circle
          className="fill-ant-container stroke-ant-primary"
          cx={motionLayout.sourceDotX}
          cy={motionLayout.sourceDotY}
          r="4"
          strokeWidth="2.5"
        />
        <motion.circle
          className="fill-ant-container stroke-ant-primary"
          cx={motionLayout.targetDotX}
          cy={motionLayout.targetDotY}
          r="4"
          strokeWidth="2.5"
        />
      </motion.svg>

      <div
        aria-hidden="true"
        className="pointer-events-none invisible absolute top-0 left-0 z-0 flex w-fit min-w-72 max-w-120 flex-col overflow-visible rounded-2 border border-ant-border bg-ant-container/95 shadow-lg backdrop-blur"
        ref={panelMeasureRef}
        style={panelMeasureStyle}
      >
        <PreviewHeader payload={payload} />

        {shouldRenderMeasuredContent(payload) && (
          <div className="min-h-0">
            <PreviewContent payload={payload} />
          </div>
        )}
      </div>

      <motion.div
        animate={active ? "open" : "closed"}
        className="absolute z-5 flex min-w-72 max-w-120 flex-col overflow-hidden rounded-2 border border-ant-border bg-ant-container/95 shadow-lg backdrop-blur"
        initial="closed"
        onPointerEnter={handlePanelPointerEnter}
        onPointerLeave={handlePanelPointerLeave}
        onPointerUp={handlePanelPointerUp}
        ref={panelRef}
        style={motionLayout.panelStyle}
        transition={PREVIEW_PANEL_TRANSITION}
        variants={PREVIEW_PANEL_VARIANTS}
      >
        <PreviewContentTransition contentKey={payloadKey}>
          <PreviewHeader payload={payload} />

          <div
            className={cn(
              "min-h-0 flex-1 cursor-text select-text overflow-hidden transition-opacity",
              {
                "opacity-60": isLoading && payload !== null,
              },
            )}
          >
            <PreviewContent payload={payload} />
          </div>
        </PreviewContentTransition>

        {isLoading && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-ant-mask/10">
            <Spin size="small" />
          </div>
        )}
      </motion.div>

      {/* 浮动复制按钮放在面板外：面板被 motion 加了 transform，fixed 子元素会以面板为
          包含块，viewport 坐标就不再成立，定位会跟着动画漂移。 */}
      <SelectionCopyButton
        containerRef={panelRef}
        enabled={active && interactive}
      />
    </div>
  );
};

function shouldRenderMeasuredContent(
  payload: ReturnType<typeof usePreviewPayload>["payload"],
) {
  if (!payload) return true;
  if (payload.kind === "image") return true;

  // 单图文件按图片渲染：同样要靠测量层给出图片的自然尺寸。
  return (
    payload.kind === "files" && payload.filesPreviewKind === "imagePreview"
  );
}

/**
 * 面板高度上限：指针停在面板上时抬到 overlay 可用高度（超长内容能滚动看完），
 * 其余情况维持默认上限。始终不低于默认上限，避免小屏被压得更矮。
 */
function resolveMaxPanelHeight(overlayHeight: number, interactive: boolean) {
  if (!interactive) return PREVIEW_PANEL_MAX_HEIGHT;

  return Math.max(
    PREVIEW_PANEL_MAX_HEIGHT,
    overlayHeight - PREVIEW_PANEL_MARGIN * 2,
  );
}

export default Preview;
