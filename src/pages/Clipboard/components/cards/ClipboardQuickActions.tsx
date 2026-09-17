import { useUnmount } from "ahooks";
import { Dropdown } from "antd";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { FC, MouseEvent, SyntheticEvent } from "react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import Tooltip from "@/components/Tooltip";
import {
  filterAvailableItemActions,
  type ItemActionLabels,
  isCopyItemAction,
  resolveItemActionPresentation,
} from "@/constants/itemActions";
import type { ClipboardItem, PasteTransform } from "@/types/clipboard";
import type { ItemAction } from "@/types/settings";
import { cn } from "@/utils/cn";

/** 直接平铺展示的动作数上限，超出折叠进「…」菜单。 */
const INLINE_ACTIONS_LIMIT = 3;

interface ClipboardQuickActionsProps {
  item: ClipboardItem;
  labels?: ItemActionLabels;
  onCleanupPaste?: (transform: PasteTransform) => Promise<void> | void;
  onQuickAction?: (action: ItemAction) => Promise<void> | void;
  quickActions: ItemAction[];
  visible: boolean;
}

interface QuickActionButtonProps {
  action: ItemAction;
  isFavorite: boolean;
  hasNote: boolean;
  isPinned: boolean;
  labels: ItemActionLabels;
  onQuickAction: (action: ItemAction) => Promise<void> | void;
  tabIndex: 0 | -1;
}

/**
 * 卡片 meta 右侧：未 hover 时快捷动作宽度归零且不带间距，时间戳贴在最右上角；
 * hover 时动作从右侧弹入，把时间戳顶向左侧并降为次级透明度。
 * 动作超过 [`INLINE_ACTIONS_LIMIT`] 个时前 3 个平铺、其余折叠进「…」菜单。
 */
const ClipboardQuickActions: FC<ClipboardQuickActionsProps> = (props) => {
  const { item, labels, onCleanupPaste, onQuickAction, quickActions, visible } =
    props;
  const shouldReduceMotion = useReducedMotion();
  const availableActions = filterAvailableItemActions(quickActions, item);
  const enabled =
    availableActions.length > 0 && Boolean(labels && onQuickAction);
  const cleanupAvailable =
    item.kind === "text" && Boolean(labels && onQuickAction && onCleanupPaste);
  const actionsVisible = visible && (enabled || cleanupAvailable);
  const tabIndex = actionsVisible ? 0 : -1;
  const actionTransition = {
    duration: shouldReduceMotion ? 0 : 0.16,
    ease: "easeOut",
  } as const;
  const inlineActions = availableActions.slice(0, INLINE_ACTIONS_LIMIT);
  const overflowActions = availableActions.slice(INLINE_ACTIONS_LIMIT);

  return (
    <div className="flex h-6 min-w-0 shrink-0 items-center justify-end">
      <span
        className={cn(
          "min-w-0 truncate transition-all duration-150 ease-out motion-reduce:transition-none",
          {
            "opacity-40": actionsVisible,
          },
        )}
      >
        {item.displayCreatedAt ?? item.createdAt}
      </span>

      {labels && onQuickAction && (enabled || cleanupAvailable) ? (
        <div
          aria-hidden={!actionsVisible}
          className={cn(
            "pointer-events-none flex items-center gap-0 opacity-0 transition-all duration-150 ease-out motion-reduce:transition-none",
            {
              "pointer-events-auto gap-0.5 pl-1 opacity-100": actionsVisible,
            },
          )}
        >
          <AnimatePresence initial={false} mode="popLayout">
            {inlineActions.map((action) => {
              return (
                <motion.span
                  animate={{
                    opacity: actionsVisible ? 1 : 0,
                    scale: 1,
                    width: actionsVisible ? "1.25rem" : 0,
                    x: 0,
                  }}
                  className="flex overflow-hidden"
                  exit={{
                    opacity: 0,
                    scale: shouldReduceMotion ? 1 : 0.9,
                    width: 0,
                    x: shouldReduceMotion ? 0 : 4,
                  }}
                  initial={{ opacity: 0, scale: 0.9, width: 0, x: 4 }}
                  key={action}
                  layout
                  transition={actionTransition}
                >
                  <QuickActionButton
                    action={action}
                    hasNote={Boolean(item.note)}
                    isFavorite={item.isFavorite}
                    isPinned={item.isPinned}
                    labels={labels}
                    onQuickAction={onQuickAction}
                    tabIndex={tabIndex}
                  />
                </motion.span>
              );
            })}
          </AnimatePresence>

          {actionsVisible && overflowActions.length > 0 ? (
            <OverflowActionsMenu
              actions={overflowActions}
              isFavorite={item.isFavorite}
              isPinned={item.isPinned}
              labels={labels}
              onQuickAction={onQuickAction}
            />
          ) : null}

          {cleanupAvailable && onCleanupPaste ? (
            <CleanupPasteMenu
              onCleanupPaste={onCleanupPaste}
              tabIndex={tabIndex}
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
};

interface OverflowActionsMenuProps {
  actions: ItemAction[];
  isFavorite: boolean;
  isPinned: boolean;
  labels: ItemActionLabels;
  onQuickAction: (action: ItemAction) => Promise<void> | void;
}

/**
 * 折叠的溢出动作「…」菜单：antd Dropdown，菜单项复用动作的图标与文案。
 */
const OverflowActionsMenu: FC<OverflowActionsMenuProps> = (props) => {
  const { actions, isFavorite, isPinned, labels, onQuickAction } = props;

  const menuItems = actions.map((action) => {
    const presentation = resolveItemActionPresentation(action, labels, {
      isFavorite: action === "star" && isFavorite,
      isPinned: action === "pinItem" && isPinned,
    });

    return {
      icon: (
        <i aria-hidden="true" className={cn(presentation.icon, "text-sm")} />
      ),
      key: action,
      label: presentation.label,
    };
  });

  return (
    <Dropdown
      menu={{
        items: menuItems,
        onClick: ({ key }) => {
          onQuickAction(key as ItemAction);
        },
      }}
      trigger={["click"]}
    >
      <button
        aria-label={labels.more}
        className="flex size-5 items-center justify-center rounded-1.5 border-0 bg-transparent text-ant-secondary transition-colors hover:bg-ant-fill-tertiary hover:text-ant-text motion-reduce:transition-none"
        onClick={(event) => {
          event.stopPropagation();
        }}
        onPointerDown={(event) => {
          event.stopPropagation();
        }}
        tabIndex={0}
        type="button"
      >
        <i aria-hidden="true" className="i-lucide:ellipsis text-sm" />
      </button>
    </Dropdown>
  );
};

export default ClipboardQuickActions;

/** 「清理粘贴」支持的 5 种变换，与 Rust `PasteTransform` 同序。 */
const CLEANUP_PASTE_TRANSFORMS: PasteTransform[] = [
  "stripNewlines",
  "trimLines",
  "trimWhitespace",
  "upperCase",
  "lowerCase",
];

interface CleanupPasteMenuProps {
  onCleanupPaste: (transform: PasteTransform) => Promise<void> | void;
  tabIndex: 0 | -1;
}

/**
 * 文本条目的「清理粘贴」下拉菜单：变换后走纯文本写回且不进历史。
 * 独立于用户可配置的悬停快捷动作，只按条目类型（文本）显隐。
 */
const CleanupPasteMenu: FC<CleanupPasteMenuProps> = (props) => {
  const { onCleanupPaste, tabIndex } = props;
  const { t } = useTranslation("clipboard");

  const menuItems = CLEANUP_PASTE_TRANSFORMS.map((transform) => {
    return {
      key: transform,
      label: t(`cleanupPaste.${transform}`),
    };
  });

  // 与 OverflowActionsMenu 同构：Dropdown 直包 button，外层不再套 Tooltip。
  // Tooltip 套 Dropdown 会让双层浮层同时 clone 同一个 trigger，在虚拟列表里
  // 诱发 Maximum update depth（hover 测量循环经 Virtuoso 回调放大）；hover
  // 提示改用原生 title，零 JS 浮层。
  return (
    <Dropdown
      menu={{
        items: menuItems,
        onClick: ({ key }) => {
          onCleanupPaste(key as PasteTransform);
        },
      }}
      trigger={["click"]}
    >
      <button
        aria-label={t("cleanupPaste.title")}
        className="flex size-5 items-center justify-center rounded-1.5 border-0 bg-transparent text-ant-secondary transition-colors hover:bg-ant-fill-tertiary hover:text-ant-text motion-reduce:transition-none"
        onClick={(event) => {
          event.stopPropagation();
        }}
        onPointerDown={(event) => {
          event.stopPropagation();
        }}
        tabIndex={tabIndex}
        title={t("cleanupPaste.title")}
        type="button"
      >
        <i aria-hidden="true" className="i-lucide:eraser text-sm" />
      </button>
    </Dropdown>
  );
};

/**
 * 单个 hover 快捷动作按钮；按下时阻止事件冒泡，避免触发卡片点击或自动粘贴。
 */
const QuickActionButton: FC<QuickActionButtonProps> = (props) => {
  const {
    action,
    hasNote,
    isFavorite,
    isPinned,
    labels,
    onQuickAction,
    tabIndex,
  } = props;
  const [copied, setCopied] = useState(false);
  const resetTimerRef = useRef<number | null>(null);
  const activeFavorite = action === "star" && isFavorite;
  const activeNote = action === "note" && hasNote;
  const activePinned = action === "pinItem" && isPinned;
  const activePrimary = activeNote || activePinned;
  const copyAction = isCopyItemAction(action);
  const presentation = resolveItemActionPresentation(action, labels, {
    copied,
    isFavorite: activeFavorite,
    isPinned: activePinned,
  });

  const stopQuickActionEvent = (event: SyntheticEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
  };

  const clearCopiedResetTimer = () => {
    if (resetTimerRef.current === null) return;

    window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = null;
  };

  const startCopiedFeedback = () => {
    setCopied(true);
    clearCopiedResetTimer();
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1000);
  };

  useUnmount(() => {
    clearCopiedResetTimer();
  });

  const handleClick = async (event: MouseEvent<HTMLButtonElement>) => {
    stopQuickActionEvent(event);

    if (copyAction && copied) return;

    await onQuickAction(action);

    if (!copyAction) return;

    startCopiedFeedback();
  };

  return (
    <Tooltip title={presentation.label}>
      <button
        aria-label={presentation.label}
        className={cn(
          "flex size-5 items-center justify-center rounded-1.5 border-0 bg-transparent text-ant-secondary transition-colors hover:bg-ant-fill-tertiary hover:text-ant-text motion-reduce:transition-none",
          {
            "text-ant-error hover:text-ant-error": presentation.danger,
            "text-ant-primary hover:text-ant-primary": activePrimary,
            "text-ant-success hover:text-ant-success": copied,
            "text-ant-warning hover:text-ant-warning": activeFavorite,
          },
        )}
        onAuxClick={stopQuickActionEvent}
        onClick={handleClick}
        onContextMenu={stopQuickActionEvent}
        onDoubleClick={stopQuickActionEvent}
        onMouseDown={stopQuickActionEvent}
        onPointerDown={stopQuickActionEvent}
        tabIndex={tabIndex}
        type="button"
      >
        <i aria-hidden="true" className={cn(presentation.icon, "text-sm")} />
      </button>
    </Tooltip>
  );
};
