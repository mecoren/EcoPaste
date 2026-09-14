import { useUnmount } from "ahooks";
import { Dropdown } from "antd";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { FC, MouseEvent, SyntheticEvent } from "react";
import { useRef, useState } from "react";
import Tooltip from "@/components/Tooltip";
import {
  filterAvailableItemActions,
  type ItemActionLabels,
  isCopyItemAction,
  resolveItemActionPresentation,
} from "@/constants/itemActions";
import type { ClipboardItem } from "@/types/clipboard";
import type { ItemAction } from "@/types/settings";
import { cn } from "@/utils/cn";

/** 直接平铺展示的动作数上限，超出折叠进「…」菜单。 */
const INLINE_ACTIONS_LIMIT = 3;

interface ClipboardQuickActionsProps {
  item: ClipboardItem;
  labels?: ItemActionLabels;
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
 * 卡片 meta 右侧：时间戳常驻（hover 时降为次级并收缩），快捷动作从其右侧
 * 弹入——hover 期间时间信息不再消失。动作超过 [`INLINE_ACTIONS_LIMIT`] 个时
 * 前 3 个平铺、其余折叠进「…」菜单。
 */
const ClipboardQuickActions: FC<ClipboardQuickActionsProps> = (props) => {
  const { item, labels, onQuickAction, quickActions, visible } = props;
  const shouldReduceMotion = useReducedMotion();
  const availableActions = filterAvailableItemActions(quickActions, item);
  const enabled =
    availableActions.length > 0 && Boolean(labels && onQuickAction);
  const actionsVisible = visible && enabled;
  const tabIndex = actionsVisible ? 0 : -1;
  const actionTransition = {
    duration: shouldReduceMotion ? 0 : 0.16,
    ease: "easeOut",
  } as const;
  const inlineActions = availableActions.slice(0, INLINE_ACTIONS_LIMIT);
  const overflowActions = availableActions.slice(INLINE_ACTIONS_LIMIT);

  return (
    <div className="flex h-6 min-w-0 shrink-0 items-center justify-end gap-1">
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

      {enabled && onQuickAction && labels ? (
        <div
          aria-hidden={!actionsVisible}
          className={cn(
            "pointer-events-none flex items-center gap-0.5 opacity-0 transition-all duration-150 ease-out motion-reduce:transition-none",
            {
              "pointer-events-auto opacity-100": actionsVisible,
            },
          )}
        >
          <AnimatePresence initial={false} mode="popLayout">
            {inlineActions.map((action) => {
              return (
                <motion.span
                  animate={{ opacity: 1, scale: 1, width: "1.25rem", x: 0 }}
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

          {overflowActions.length > 0 ? (
            <OverflowActionsMenu
              actions={overflowActions}
              isFavorite={item.isFavorite}
              isPinned={item.isPinned}
              labels={labels}
              onQuickAction={onQuickAction}
              visible={actionsVisible}
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
  visible: boolean;
}

/**
 * 折叠的溢出动作「…」菜单：antd Dropdown，菜单项复用动作的图标与文案。
 */
const OverflowActionsMenu: FC<OverflowActionsMenuProps> = (props) => {
  const { actions, isFavorite, isPinned, labels, onQuickAction, visible } =
    props;

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
        className={cn(
          "flex size-5 items-center justify-center rounded-1.5 border-0 bg-transparent text-ant-secondary transition-colors hover:bg-ant-fill-tertiary hover:text-ant-text motion-reduce:transition-none",
          { invisible: !visible },
        )}
        onClick={(event) => {
          event.stopPropagation();
        }}
        onPointerDown={(event) => {
          event.stopPropagation();
        }}
        tabIndex={visible ? 0 : -1}
        type="button"
      >
        <i aria-hidden="true" className="i-lucide:ellipsis text-sm" />
      </button>
    </Dropdown>
  );
};

export default ClipboardQuickActions;

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
