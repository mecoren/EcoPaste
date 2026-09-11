import { useMount } from "ahooks";
import { Input, type InputRef } from "antd";
import type { TFunction } from "i18next";
import type {
  ChangeEvent,
  CompositionEvent,
  Dispatch,
  FC,
  MouseEvent,
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
  RefObject,
  SetStateAction,
} from "react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSnapshot } from "valtio";
import {
  createClipboardGroup,
  deleteClipboardGroup,
  listClipboardGroups,
  openPreferenceWithHighlight,
  updateClipboardGroup,
} from "@/commands";
import AssetImage from "@/components/AssetImage";
import ClipboardGroupIcon from "@/components/ClipboardGroupIcon";
import ClipboardGroupModal from "@/components/ClipboardGroupModal";
import Dropdown, { type DropdownMenuItems } from "@/components/Dropdown";
import KeyHint from "@/components/KeyHint";
import ScrollArea from "@/components/ScrollArea";
import Tooltip from "@/components/Tooltip";
import { TAURI_EVENT } from "@/constants/events";
import { prepareClipboardWindowEditableFocus } from "@/hooks/useClipboardWindowEditableFocus";
import { useKeyboardEvent } from "@/hooks/useKeyboardEvent";
import { useTauriListen } from "@/hooks/useTauriListen";
import { clipboardViewState } from "@/stores/clipboardView";
import { preloadSourceApps, sourceAppsState } from "@/stores/sourceApps";
import type {
  ClipboardApp,
  ClipboardCategory,
  ClipboardGroupIcon as ClipboardGroupIconValue,
  ClipboardGroupInput,
  ClipboardGroupRecord,
  ClipboardRange,
} from "@/types/clipboard";
import { SOURCE_APP_NONE } from "@/types/clipboard";
import { cn } from "@/utils/cn";
import { getModalApi } from "@/utils/feedback";

type GroupModalMode = "create" | "edit";
type MoreMenuAction = "manageGroups" | "newGroup";
type GroupMenuAction = "delete" | "edit" | "hide";
type MoreMenuGroupKey = `group:${string}`;

interface RangeGroupOption {
  labelKey: string;
  value: ClipboardRange;
  icon: ClipboardGroupIconValue;
}

interface CategoryGroupOption {
  labelKey: string;
  value: ClipboardCategory;
  icon: ClipboardGroupIconValue;
}

interface SourceAppRowProps {
  /** 键盘导航激活项：渲染高亮并挂 ref 供滚动到可视区。 */
  active: boolean;
  icon?: ReactNode;
  itemRef?: RefObject<HTMLButtonElement | null>;
  onClick: () => void;
  selected: boolean;
  title: string;
}

interface SourceAppPopupProps {
  apps: readonly ClipboardApp[];
  onClose: () => void;
  onToggleAppFilter: (appId: string) => void;
  selectedAppId: string | null;
}

interface OverflowGroupMenuLabelProps {
  menuItems: DropdownMenuItems;
  onContext: (record: ClipboardGroupRecord) => void;
  onMenuClick: (info: { key: string }) => void;
  record: ClipboardGroupRecord;
}

interface GroupSeparatorProps {
  separatorRef?: RefObject<HTMLSpanElement | null>;
}

const RANGE_GROUP_OPTIONS: RangeGroupOption[] = [
  { icon: "i-lets-icons:widget", labelKey: "groups.all", value: "all" },
  {
    icon: "i-lets-icons:star",
    labelKey: "groups.favorite",
    value: "favorite",
  },
];

const CATEGORY_GROUP_OPTIONS: CategoryGroupOption[] = [
  { icon: "i-lets-icons:file-dock", labelKey: "groups.text", value: "text" },
  { icon: "i-lets-icons:img-box", labelKey: "groups.image", value: "image" },
  {
    icon: "i-lets-icons:folder-file-alt",
    labelKey: "groups.files",
    value: "files",
  },
];

const GROUP_MENU_ACTION = {
  DELETE: "delete",
  EDIT: "edit",
  HIDE: "hide",
} as const satisfies Record<string, GroupMenuAction>;

const MORE_MENU_ACTION = {
  MANAGE_GROUPS: "manageGroups",
  NEW_GROUP: "newGroup",
} as const satisfies Record<string, MoreMenuAction>;

const CUSTOM_GROUPS_SETTING_ID = "organizing.customGroups";

const GROUP_BUTTON_BASE_CLASS =
  "flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-1.5 border-0 bg-transparent p-0 transition-colors";
const GROUP_ICON_BUTTON_CLASS = GROUP_BUTTON_BASE_CLASS;
const GROUP_BUTTON_WIDTH = 24;
const GROUP_BUTTON_GAP = 4;
const GROUP_SEPARATOR_MARGIN = 4;

/**
 * 来源应用弹层对齐策略：小屏窗口（360px）下默认 bottomLeft 会让 224px 宽的弹层
 * 右溢视口；开启 shiftX 让 rc-align 把右溢平移回可视区，4px 为按钮贴近边界的保护值。
 */
const SOURCE_APP_POPUP_ALIGN = {
  overflow: { adjustX: true, adjustY: true, shiftX: 4, shiftY: true },
} as const satisfies Record<string, unknown>;

/**
 * Header 下方的分组筛选栏：内置类型分组 + 自定义分组入口。
 */
const Group: FC = () => {
  const { t } = useTranslation(["clipboard", "common"]);
  const { category, groupId, range, sourceAppId } =
    useSnapshot(clipboardViewState);
  const { apps: sourceApps } = useSnapshot(sourceAppsState);

  const [customGroups, setCustomGroups] = useState<ClipboardGroupRecord[]>([]);
  const [modalOpen, setModalOpen] = useState(false);
  const [modalMode, setModalMode] = useState<GroupModalMode>("create");
  const [sourceAppPopupOpen, setSourceAppPopupOpen] = useState(false);
  const [visibleCustomGroupCount, setVisibleCustomGroupCount] = useState(
    Number.POSITIVE_INFINITY,
  );
  const [editingGroup, setEditingGroup] = useState<ClipboardGroupRecord | null>(
    null,
  );
  const toolbarRef = useRef<HTMLDivElement>(null);
  const customGroupAnchorRef = useRef<HTMLSpanElement>(null);
  const contextGroupRef = useRef<ClipboardGroupRecord | null>(null);
  const deleteGroupRef = useRef<ClipboardGroupRecord | null>(null);

  const visibleCustomGroups = customGroups.filter((record) => {
    return !record.isHidden;
  });
  const inlineCustomGroups = visibleCustomGroups.slice(
    0,
    visibleCustomGroupCount,
  );
  const overflowCustomGroups = visibleCustomGroups.slice(
    visibleCustomGroupCount,
  );

  /**
   * 从 Rust 拉取自定义分组。
   */
  const loadGroups = async () => {
    const groups = await listClipboardGroups();

    setCustomGroups(groups);
    scheduleVisibleCustomGroupCountUpdate(
      toolbarRef,
      customGroupAnchorRef,
      setVisibleCustomGroupCount,
      groups.filter((record) => {
        return !record.isHidden;
      }).length,
    );
    ensureSelectedGroupStillExists(groups);
  };

  /**
   * 首次挂载时拉取分组。
   */
  useMount(() => {
    void loadGroups();
    void preloadSourceApps();
  });

  /** 当前选中的来源应用记录（哨兵值除外时用于按钮展示）。 */
  const selectedSourceApp =
    sourceApps.find((app) => {
      return app.id === sourceAppId;
    }) ?? null;

  /**
   * 其他窗口或命令修改分组后刷新本地列表。
   */
  const handleGroupsUpdated = () => {
    void loadGroups();
  };

  useTauriListen(TAURI_EVENT.CLIPBOARD_GROUPS_UPDATED, handleGroupsUpdated);

  /**
   * 容器尺寸变化时重新测量溢出状态。
   */
  useEffect(() => {
    const toolbar = toolbarRef.current;
    const customAnchor = customGroupAnchorRef.current;
    if (!toolbar || !customAnchor) return;

    const updateVisibleCustomGroupCount = () => {
      commitVisibleCustomGroupCount(
        toolbar,
        customAnchor,
        setVisibleCustomGroupCount,
        visibleCustomGroups.length,
      );
    };

    const observer = new ResizeObserver(updateVisibleCustomGroupCount);
    observer.observe(toolbar);
    updateVisibleCustomGroupCount();

    return () => {
      observer.disconnect();
    };
  }, [visibleCustomGroups.length]);

  /**
   * 切换范围；范围必须始终保留一个选中项。
   */
  const selectRange = (value: ClipboardRange) => {
    clipboardViewState.range = value;
  };

  /**
   * 切换分类；再次点击当前分类时取消。
   */
  const toggleCategory = (value: ClipboardCategory) => {
    clipboardViewState.category =
      clipboardViewState.category === value ? null : value;
  };

  /**
   * 切换到自定义分组；再次点击当前分组时取消。
   */
  const toggleCustomGroup = (id: string) => {
    clipboardViewState.groupId = clipboardViewState.groupId === id ? null : id;
  };

  /**
   * 切换来源应用筛选；再次点击当前应用时取消。
   */
  const handleSourceAppToggleFilter = (appId: string) => {
    clipboardViewState.sourceAppId =
      clipboardViewState.sourceAppId === appId ? null : appId;
  };

  /**
   * 来源应用下拉开合受控：点击外部 / 选中行后由这里统一收口关闭，
   * 关闭即销毁弹层（destroyOnHidden），搜索词与高亮自然重置。
   */
  const handleSourceAppOpenChange = (open: boolean) => {
    setSourceAppPopupOpen(open);
  };

  /**
   * 选中行后关闭弹层，恢复背景列表的正常键盘语义。
   */
  const handleSourceAppClose = () => {
    setSourceAppPopupOpen(false);
  };

  /**
   * 点击分组按钮时根据 data 属性切换筛选。
   */
  const handleGroupClick = (event: MouseEvent<HTMLButtonElement>) => {
    const type = event.currentTarget.dataset.type;
    const value = event.currentTarget.dataset.value;
    const nextGroupId = event.currentTarget.dataset.groupId;

    if (nextGroupId) {
      toggleCustomGroup(nextGroupId);
      return;
    }

    if (type === "range" && isRangeGroup(value)) {
      selectRange(value);
      return;
    }

    if (type === "category" && isCategoryGroup(value)) {
      toggleCategory(value);
    }
  };

  /**
   * 记录右键菜单所属分组。
   */
  const handleCustomGroupContextMenu = (
    event: MouseEvent<HTMLButtonElement>,
  ) => {
    const nextGroupId = event.currentTarget.dataset.groupId;
    if (!nextGroupId) return;

    contextGroupRef.current =
      customGroups.find((record) => {
        return record.id === nextGroupId;
      }) ?? null;
  };

  /**
   * 处理分组栏快捷键：Cmd/Ctrl+Q 切换范围，左右键切分类，Tab / Shift+Tab 仅在可见自定义分组间循环。
   */
  const handleKeyDown = (event: KeyboardEvent) => {
    const eventModifierPressed = event.metaKey || event.ctrlKey;

    if (eventModifierPressed && event.key.toLowerCase() === "q") {
      event.preventDefault();
      toggleRange();

      return;
    }

    if (
      (event.key === "ArrowLeft" || event.key === "ArrowRight") &&
      !shouldUseNativeHorizontalNavigation(event)
    ) {
      event.preventDefault();
      selectAdjacentCategory(event.key === "ArrowLeft" ? -1 : 1);

      return;
    }

    if (event.key !== "Tab") return;

    event.preventDefault();

    const nextGroupId = selectAdjacentCustomGroup(
      visibleCustomGroups,
      groupId,
      event.shiftKey,
    );

    if (!nextGroupId) return;

    toggleCustomGroup(nextGroupId);
  };

  useKeyboardEvent("keydown", handleKeyDown);

  /**
   * 在全部 / 收藏范围之间循环切换，不影响分类与自定义分组筛选。
   */
  const toggleRange = () => {
    clipboardViewState.range =
      clipboardViewState.range === "all" ? "favorite" : "all";
  };

  /**
   * 按方向键在固定分类序列内循环；未选分类时从方向对应的端点进入。
   */
  const selectAdjacentCategory = (direction: -1 | 1) => {
    const options = CATEGORY_GROUP_OPTIONS.map((option) => {
      return option.value;
    });
    const currentCategory = clipboardViewState.category;
    const current = currentCategory ? options.indexOf(currentCategory) : -1;
    const startIndex = direction === 1 ? -1 : options.length;
    const nextIndex =
      (current === -1 ? startIndex + direction : current + direction) %
      options.length;
    const normalizedIndex = (nextIndex + options.length) % options.length;

    clipboardViewState.category = options[normalizedIndex];
  };

  /**
   * 打开新增分组弹框。
   */
  const openCreateModal = () => {
    setModalMode("create");
    setEditingGroup(null);
    setModalOpen(true);
  };

  /**
   * 打开编辑分组弹框。
   */
  const openEditModal = (record: ClipboardGroupRecord) => {
    setModalMode("edit");
    setEditingGroup(record);
    setModalOpen(true);
  };

  /**
   * 关闭新增 / 编辑分组弹框。
   */
  const closeModal = () => {
    setModalOpen(false);
    setEditingGroup(null);
  };

  /**
   * 保存分组弹框内容。
   */
  const handleModalSubmit = async (input: ClipboardGroupInput) => {
    if (modalMode === "create") {
      await createClipboardGroup(input);
      closeModal();
      return;
    }

    if (!editingGroup) return;

    await updateClipboardGroup(editingGroup.id, input);
    closeModal();
  };

  /**
   * 执行自定义分组右键菜单动作。
   */
  const handleGroupMenuClick = (info: { key: string }) => {
    const record = contextGroupRef.current;
    if (!record) return;

    const action = parseGroupMenuAction(info.key);
    if (!action) return;

    if (action === GROUP_MENU_ACTION.EDIT) {
      openEditModal(record);
      return;
    }

    if (action === GROUP_MENU_ACTION.HIDE) {
      void updateClipboardGroup(record.id, {
        icon: record.icon,
        isHidden: true,
        name: record.name,
      });
      return;
    }

    if (action === GROUP_MENU_ACTION.DELETE) {
      requestDeleteGroup(record);
    }
  };

  /**
   * 执行新增分组动作。
   */
  const handleCreateGroupAction = () => {
    openCreateModal();
  };

  /**
   * 打开偏好设置并定位到自定义分组管理项。
   */
  const openGroupPreference = async () => {
    await openPreferenceWithHighlight(CUSTOM_GROUPS_SETTING_ID);
  };

  /**
   * 执行更多菜单动作：新增 / 管理分组，或切换到溢出的自定义分组。
   */
  const handleMoreMenuClick = async (info: { key: string }) => {
    const action = parseMoreMenuAction(info.key);
    if (action === MORE_MENU_ACTION.NEW_GROUP) {
      handleCreateGroupAction();
      return;
    }

    if (action === MORE_MENU_ACTION.MANAGE_GROUPS) {
      await openGroupPreference();
      return;
    }

    const id = parseMoreMenuGroupId(info.key);
    if (!id) return;

    toggleCustomGroup(id);
  };

  /**
   * 弹出删除确认框。
   */
  const requestDeleteGroup = (record: ClipboardGroupRecord) => {
    deleteGroupRef.current = record;

    getModalApi().confirm({
      centered: true,
      content: (
        <span className="text-ant-secondary text-sm">
          {t("clipboard:groups.deleteConfirmDescription", {
            group: record.name,
          })}
        </span>
      ),
      okButtonProps: { danger: true },
      okText: t("common:actions.delete"),
      onOk: confirmDeleteGroup,
      title: t("clipboard:groups.delete"),
    });
  };

  /**
   * 确认删除当前待删除分组。
   */
  const confirmDeleteGroup = async () => {
    const record = deleteGroupRef.current;
    if (!record) return;

    await deleteClipboardGroup(record.id);

    if (clipboardViewState.groupId === record.id) {
      clipboardViewState.groupId = null;
    }

    deleteGroupRef.current = null;
  };

  const groupMenuItems = buildGroupActionMenuItems(t);
  const createMenuItems = buildCreateMenuItems(t);

  /**
   * 记录溢出菜单中右键菜单所属分组。
   */
  const handleOverflowGroupContext = (record: ClipboardGroupRecord) => {
    contextGroupRef.current = record;
  };

  const moreMenuItems = buildMoreMenuItems(
    overflowCustomGroups,
    groupMenuItems,
    handleGroupMenuClick,
    handleOverflowGroupContext,
    t,
  );
  const moreMenuSelectedKeys = groupId ? [buildMoreMenuGroupKey(groupId)] : [];
  const moreButtonSelected = overflowCustomGroups.some((record) => {
    return record.id === groupId;
  });

  /**
   * 渲染溢出分组菜单按钮。
   */
  const renderMoreButton = () => {
    if (overflowCustomGroups.length === 0) return null;

    return (
      <Dropdown
        menu={{
          items: moreMenuItems,
          onClick: handleMoreMenuClick,
          selectedKeys: moreMenuSelectedKeys,
        }}
        tooltip={t("clipboard:groups.more")}
        trigger={["click"]}
      >
        <button
          className={cn(GROUP_BUTTON_BASE_CLASS, {
            "bg-ant-primary text-ant-light-solid": moreButtonSelected,
            "text-ant-secondary hover:bg-ant-fill-tertiary":
              !moreButtonSelected,
          })}
          type="button"
        >
          <KeyHint hintKey="N" onKeyPress={handleCreateGroupAction}>
            <i aria-hidden className="i-lucide:more-horizontal text-sm!" />
          </KeyHint>
        </button>
      </Dropdown>
    );
  };

  /**
   * 渲染独立新增按钮；存在溢出菜单时由菜单内新增入口承接。
   */
  const renderCreateButton = () => {
    if (overflowCustomGroups.length > 0) return null;

    return (
      <Dropdown
        menu={{
          items: createMenuItems,
          onClick: handleMoreMenuClick,
        }}
        tooltip={t("clipboard:groups.add")}
        trigger={["contextMenu"]}
      >
        <button
          className={cn(
            GROUP_BUTTON_BASE_CLASS,
            "text-ant-secondary hover:bg-ant-fill-tertiary",
          )}
          onClick={handleCreateGroupAction}
          type="button"
        >
          <KeyHint hintKey="N" onKeyPress={handleCreateGroupAction}>
            <i aria-hidden className="i-lucide:plus text-sm!" />
          </KeyHint>
        </button>
      </Dropdown>
    );
  };

  /**
   * 渲染范围按钮。
   */
  const renderRangeButton = ({ labelKey, value, icon }: RangeGroupOption) => {
    const selected = range === value;
    const nextRange =
      range === "all" ? "favorite" : range === "favorite" ? "all" : void 0;
    const showShortcutHint = nextRange === value;

    return renderFilterButton({
      icon,
      label: t(`clipboard:${labelKey}`),
      selected,
      showShortcutHint,
      type: "range",
      value,
    });
  };

  /**
   * 渲染分类按钮。
   */
  const renderCategoryButton = ({
    labelKey,
    value,
    icon,
  }: CategoryGroupOption) => {
    const selected = category === value;

    return renderFilterButton({
      icon,
      label: t(`clipboard:${labelKey}`),
      selected,
      type: "category",
      value,
    });
  };

  /**
   * 渲染来源应用筛选按钮 + 可搜索的应用下拉菜单。
   */
  const renderSourceAppButton = () => {
    const selectedApp =
      sourceAppId === SOURCE_APP_NONE ? void 0 : selectedSourceApp;
    const tooltipTitle =
      sourceAppId === SOURCE_APP_NONE
        ? t("clipboard:groups.sourceAppNone")
        : (selectedApp?.name ?? t("clipboard:groups.sourceApp"));

    const renderSourceAppPopup = () => {
      return (
        <SourceAppPopup
          apps={sourceApps}
          onClose={handleSourceAppClose}
          onToggleAppFilter={handleSourceAppToggleFilter}
          selectedAppId={sourceAppId}
        />
      );
    };

    return (
      <Dropdown
        align={SOURCE_APP_POPUP_ALIGN}
        destroyOnHidden
        onOpenChange={handleSourceAppOpenChange}
        open={sourceAppPopupOpen}
        popupRender={renderSourceAppPopup}
        tooltip={tooltipTitle}
        trigger={["click"]}
      >
        <button
          className={cn(GROUP_ICON_BUTTON_CLASS, {
            "bg-ant-primary text-ant-light-solid": sourceAppId !== null,
            "text-ant-secondary hover:bg-ant-fill-tertiary":
              sourceAppId === null,
          })}
          type="button"
        >
          {selectedApp?.iconPath ? (
            <AssetImage
              alt={selectedApp.name}
              className="size-4 rounded-0.5"
              src={selectedApp.iconPath}
            />
          ) : (
            <i aria-hidden className="i-lucide:app-window text-sm!" />
          )}
        </button>
      </Dropdown>
    );
  };

  /**
   * 渲染单个筛选按钮。
   */
  const renderFilterButton = (options: {
    icon: ClipboardGroupIconValue;
    label: string;
    selected: boolean;
    showShortcutHint?: boolean;
    type: "category" | "range";
    value: ClipboardCategory | ClipboardRange;
  }) => {
    const { icon, label, selected, showShortcutHint, type, value } = options;

    return (
      <Tooltip key={`${type}:${value}`} title={label}>
        <button
          className={cn(GROUP_ICON_BUTTON_CLASS, {
            "bg-ant-primary text-ant-light-solid": selected,
            "text-ant-secondary hover:bg-ant-fill-tertiary": !selected,
          })}
          data-type={type}
          data-value={value}
          onClick={handleGroupClick}
          type="button"
        >
          {showShortcutHint ? (
            <KeyHint hintKey="Q">
              <ClipboardGroupIcon icon={icon} selected={selected} />
            </KeyHint>
          ) : (
            <ClipboardGroupIcon icon={icon} selected={selected} />
          )}
        </button>
      </Tooltip>
    );
  };

  return (
    <>
      <div
        className="flex items-center gap-1 overflow-hidden px-3 pb-2"
        data-tauri-drag-region
        ref={toolbarRef}
      >
        {RANGE_GROUP_OPTIONS.map(renderRangeButton)}
        <GroupSeparator />
        {CATEGORY_GROUP_OPTIONS.map(renderCategoryButton)}
        {renderSourceAppButton()}
        <GroupSeparator separatorRef={customGroupAnchorRef} />

        {inlineCustomGroups.length > 0 && (
          <div className="flex min-w-0 shrink-0 items-center gap-1 overflow-hidden">
            {inlineCustomGroups.map((record) => {
              const selected = groupId === record.id;

              return (
                <Dropdown
                  key={record.id}
                  menu={{
                    items: groupMenuItems,
                    onClick: handleGroupMenuClick,
                  }}
                  tooltip={record.name}
                  trigger={["contextMenu"]}
                >
                  <button
                    className={cn(GROUP_ICON_BUTTON_CLASS, {
                      "bg-ant-primary text-ant-light-solid": selected,
                      "text-ant-secondary hover:bg-ant-fill-tertiary":
                        !selected,
                    })}
                    data-group-id={record.id}
                    onClick={handleGroupClick}
                    onContextMenu={handleCustomGroupContextMenu}
                    type="button"
                  >
                    <ClipboardGroupIcon
                      icon={record.icon}
                      selected={selected}
                    />
                  </button>
                </Dropdown>
              );
            })}
          </div>
        )}

        {renderMoreButton()}
        {renderCreateButton()}
      </div>

      <ClipboardGroupModal
        group={editingGroup}
        mode={modalMode}
        onCancel={closeModal}
        onSubmit={handleModalSubmit}
        open={modalOpen}
      />
    </>
  );
};

/**
 * 分隔范围、分类、自定义分组三段。
 */
const GroupSeparator: FC<GroupSeparatorProps> = (props) => {
  const { separatorRef } = props;

  return (
    <span
      aria-hidden
      className="mx-1 h-4 w-px shrink-0 bg-ant-split"
      ref={separatorRef}
    />
  );
};

/**
 * 来源应用下拉弹层：非受控搜索框（IME 组合期不回灌）+ 自绘行列表。
 * 弹层随 Dropdown 打开重新挂载，搜索词与键盘高亮自然重置，无需手动清理。
 * Windows 剪贴板窗口默认不可聚焦：挂载后先恢复窗口可聚焦再聚焦搜索框，
 * 顺序颠倒会让 IME 关联到未激活窗口，拼音组合输入卡死。
 */
const SourceAppPopup: FC<SourceAppPopupProps> = (props) => {
  const { apps, onClose, onToggleAppFilter, selectedAppId } = props;
  const { t } = useTranslation("clipboard");

  const inputRef = useRef<InputRef>(null);
  const composingRef = useRef(false);
  const activeItemRef = useRef<HTMLButtonElement | null>(null);
  const [keyword, setKeyword] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);

  const query = keyword.trim().toLocaleLowerCase();
  const filteredApps = apps.filter((app) => {
    return app.name.toLocaleLowerCase().includes(query);
  });
  // 键盘导航顺序即行渲染顺序：无来源哨兵仅在未搜索时出现。
  const activeKeys = [
    ...(query === "" ? [SOURCE_APP_NONE] : []),
    ...filteredApps.map((app) => {
      return app.id;
    }),
  ];

  /**
   * 挂载即聚焦：先让 Rust 恢复剪贴板窗口可聚焦，再聚焦搜索框。
   */
  useMount(() => {
    const focusInput = async () => {
      await prepareClipboardWindowEditableFocus();
      inputRef.current?.focus();
    };

    void focusInput();
  });

  /**
   * 键盘高亮变化后滚动到可视区，长列表导航时不脱离视野。
   */
  // biome-ignore lint/correctness/useExhaustiveDependencies: ref 内容由 activeIndex 变化后的 render 更新，需以 activeIndex 为触发器
  useEffect(() => {
    activeItemRef.current?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  /**
   * 键盘导航：↑/↓ 在行间循环移动高亮，Enter 选中当前项，Escape 只关弹层。
   * 可打印字符不拦截，焦点始终在搜索框，输入自然落入搜索词并过滤行。
   * 全部按键阻止冒泡：背景列表的全局键盘语义（Enter 粘贴、方向键移动选中、
   * ESC 逐层退出）在弹层打开期间不应触发。
   */
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;

    if (activeKeys.length === 0) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }

      return;
    }

    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      event.stopPropagation();

      const delta = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => {
        return (current + delta + activeKeys.length) % activeKeys.length;
      });

      return;
    }

    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();

      const key = activeKeys[activeIndex];
      if (key === void 0) return;

      onToggleAppFilter(key);
      onClose();

      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
  };

  /**
   * IME 组合输入开始：暂停搜索词更新，避免拼音中间态污染过滤。
   */
  const handleCompositionStart = () => {
    composingRef.current = true;
  };

  /**
   * IME 组合输入结束：解除暂停并补发一次变更。
   */
  const handleCompositionEnd = (event: CompositionEvent<HTMLInputElement>) => {
    composingRef.current = false;
    setKeyword(event.currentTarget.value);
    setActiveIndex(0);
  };

  /**
   * 搜索词变化；组合输入期间不更新；搜索词变化后键盘高亮回到首行。
   */
  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    if (composingRef.current) return;

    setKeyword(event.target.value);
    setActiveIndex(0);
  };

  /**
   * 点击行切换筛选后关闭弹层，恢复背景列表的正常键盘语义。
   */
  const renderRow = (options: {
    active: boolean;
    icon?: ReactNode;
    key: string;
    title: string;
  }) => {
    const { active, icon, key, title } = options;

    const handleClick = () => {
      onToggleAppFilter(key);
      onClose();
    };

    return (
      <SourceAppRow
        active={active}
        icon={icon}
        itemRef={active ? activeItemRef : void 0}
        key={key}
        onClick={handleClick}
        selected={selectedAppId === key}
        title={title}
      />
    );
  };

  const listContent =
    activeKeys.length === 0 ? (
      <div className="py-1.5 text-center text-ant-secondary text-sm">
        {t("groups.sourceAppEmpty")}
      </div>
    ) : (
      <ScrollArea
        className="max-h-64"
        contentClassName="flex flex-col gap-0.5 py-0.5"
      >
        {query === "" &&
          renderRow({
            active: activeIndex === 0,
            key: SOURCE_APP_NONE,
            title: t("groups.sourceAppNone"),
          })}

        {filteredApps.map((app, index) => {
          const itemIndex = index + (query === "" ? 1 : 0);

          return renderRow({
            active: activeIndex === itemIndex,
            icon: (
              <AssetImage
                alt={app.name}
                className="size-4 rounded-0.5"
                src={app.iconPath}
              />
            ),
            key: app.id,
            title: app.name,
          });
        })}
      </ScrollArea>
    );

  return (
    <div className="w-56 max-w-[calc(100vw-2rem)] rounded-2 border border-ant-border-secondary bg-ant-elevated p-1 shadow-lg">
      <Input
        allowClear
        onChange={handleChange}
        onCompositionEnd={handleCompositionEnd}
        onCompositionStart={handleCompositionStart}
        onKeyDown={handleKeyDown}
        placeholder={t("groups.sourceAppSearchPlaceholder")}
        prefix={<i aria-hidden className="i-lucide:search" />}
        ref={inputRef}
        size="small"
        spellCheck={false}
      />

      {listContent}
    </div>
  );
};

/**
 * 来源应用下拉行：图标（无则省略）+ 名称；键盘激活态高亮，选中态展示勾选。
 */
const SourceAppRow: FC<SourceAppRowProps> = (props) => {
  const { active, icon, itemRef, onClick, selected, title } = props;

  return (
    <button
      className={cn(
        "flex w-full min-w-0 cursor-pointer items-center gap-2 rounded-1 border-0 bg-transparent px-1.5 py-1 text-left text-inherit transition-colors",
        active ? "bg-ant-fill-tertiary" : "hover:bg-ant-fill-tertiary",
      )}
      onClick={onClick}
      ref={itemRef}
      type="button"
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{title}</span>
      {selected ? (
        <i aria-hidden className="i-lucide:check shrink-0 text-sm!" />
      ) : null}
    </button>
  );
};

/**
 * 溢出菜单里的分组行：左键选择分组，右键打开同一套分组管理菜单。
 */
const OverflowGroupMenuLabel: FC<OverflowGroupMenuLabelProps> = (props) => {
  const { menuItems, onContext, onMenuClick, record } = props;

  const handleContextMenu = () => {
    onContext(record);
  };

  return (
    <Dropdown
      menu={{
        items: menuItems,
        onClick: onMenuClick,
      }}
      trigger={["contextMenu"]}
    >
      <span
        className="flex min-w-28 items-center gap-2"
        onContextMenu={handleContextMenu}
        role="menuitem"
        tabIndex={-1}
      >
        <ClipboardGroupIcon icon={record.icon} inheritColor />
        <span>{record.name}</span>
      </span>
    </Dropdown>
  );
};

/**
 * 下一帧提交可见自定义分组数量；分组数据更新后等待 DOM 渲染完成再测量。
 */
function scheduleVisibleCustomGroupCountUpdate(
  toolbarRef: RefObject<HTMLDivElement | null>,
  customAnchorRef: RefObject<HTMLSpanElement | null>,
  setVisibleCustomGroupCount: Dispatch<SetStateAction<number>>,
  groupCount: number,
) {
  requestAnimationFrame(() => {
    commitVisibleCustomGroupCount(
      toolbarRef.current,
      customAnchorRef.current,
      setVisibleCustomGroupCount,
      groupCount,
    );
  });
}

/**
 * 根据自定义分组栏可用宽度写入可见分组数量。
 */
function commitVisibleCustomGroupCount(
  toolbar: HTMLDivElement | null,
  customAnchor: HTMLSpanElement | null,
  setVisibleCustomGroupCount: Dispatch<SetStateAction<number>>,
  groupCount: number,
) {
  const rawCapacity =
    toolbar && customAnchor
      ? computeCustomGroupCapacity(toolbar, customAnchor)
      : groupCount;
  const visibleCount = Math.min(groupCount, rawCapacity);

  setVisibleCustomGroupCount((current) => {
    if (current === visibleCount) return current;

    return visibleCount;
  });
}

/**
 * 按整条分组栏剩余宽度计算自定义分组可显示数量。
 */
function computeCustomGroupCapacity(
  toolbar: HTMLDivElement,
  customAnchor: HTMLSpanElement,
) {
  const toolbarRect = toolbar.getBoundingClientRect();
  const customRect = customAnchor.getBoundingClientRect();
  const customStart =
    customRect.right -
    toolbarRect.left +
    GROUP_SEPARATOR_MARGIN +
    GROUP_BUTTON_GAP;
  const actionSlotWidth = GROUP_BUTTON_GAP + GROUP_BUTTON_WIDTH;
  const availableWidth = Math.max(
    0,
    toolbar.clientWidth - customStart - actionSlotWidth,
  );

  return Math.max(
    0,
    Math.floor(
      (availableWidth + GROUP_BUTTON_GAP) /
        (GROUP_BUTTON_WIDTH + GROUP_BUTTON_GAP),
    ),
  );
}

/**
 * 构建自定义分组右键菜单；内联分组和溢出菜单分组共用这一份定义。
 */
function buildGroupActionMenuItems(
  t: TFunction<["clipboard", "common"]>,
): DropdownMenuItems {
  return [
    {
      icon: "i-lucide:pencil",
      key: GROUP_MENU_ACTION.EDIT,
      label: t("clipboard:groups.edit"),
    },
    {
      icon: "i-lucide:eye-off",
      key: GROUP_MENU_ACTION.HIDE,
      label: t("clipboard:groups.hide"),
    },
    { type: "divider" },
    {
      danger: true,
      icon: "i-lucide:trash-2",
      key: GROUP_MENU_ACTION.DELETE,
      label: t("clipboard:groups.delete"),
    },
  ];
}

/**
 * 构建新增按钮右键菜单；左键继续新增，右键提供管理入口。
 */
function buildCreateMenuItems(
  t: TFunction<["clipboard", "common"]>,
): DropdownMenuItems {
  return [
    {
      icon: "i-lucide:settings-2",
      key: MORE_MENU_ACTION.MANAGE_GROUPS,
      label: t("clipboard:groups.manage"),
    },
  ];
}

/**
 * 构建更多菜单项：新增 / 管理入口 + 溢出分组快速入口。
 */
function buildMoreMenuItems(
  groups: ClipboardGroupRecord[],
  groupMenuItems: DropdownMenuItems,
  onGroupMenuClick: (info: { key: string }) => void,
  onGroupContext: (record: ClipboardGroupRecord) => void,
  t: TFunction<["clipboard", "common"]>,
): DropdownMenuItems {
  const groupItems = groups.map((record) => {
    return {
      key: buildMoreMenuGroupKey(record.id),
      label: (
        <OverflowGroupMenuLabel
          menuItems={groupMenuItems}
          onContext={onGroupContext}
          onMenuClick={onGroupMenuClick}
          record={record}
        />
      ),
    };
  });

  if (groupItems.length === 0) {
    return [
      {
        icon: "i-lucide:plus",
        key: MORE_MENU_ACTION.NEW_GROUP,
        label: t("clipboard:groups.add"),
      },
      {
        icon: "i-lucide:settings-2",
        key: MORE_MENU_ACTION.MANAGE_GROUPS,
        label: t("clipboard:groups.manage"),
      },
    ];
  }

  return [
    ...groupItems,
    { type: "divider" },
    {
      icon: "i-lucide:plus",
      key: MORE_MENU_ACTION.NEW_GROUP,
      label: t("clipboard:groups.add"),
    },
    {
      icon: "i-lucide:settings-2",
      key: MORE_MENU_ACTION.MANAGE_GROUPS,
      label: t("clipboard:groups.manage"),
    },
  ];
}

/**
 * 解析自定义分组右键菜单动作。
 */
function parseGroupMenuAction(key: string): GroupMenuAction | null {
  const actions = Object.values(GROUP_MENU_ACTION);
  if (!actions.includes(key as GroupMenuAction)) return null;

  return key as GroupMenuAction;
}

/**
 * 解析更多菜单动作。
 */
function parseMoreMenuAction(key: string): MoreMenuAction | null {
  const actions = Object.values(MORE_MENU_ACTION);
  if (!actions.includes(key as MoreMenuAction)) return null;

  return key as MoreMenuAction;
}

/**
 * 生成更多菜单中的分组 key。
 */
function buildMoreMenuGroupKey(id: string): MoreMenuGroupKey {
  return `group:${id}`;
}

/**
 * 从更多菜单 key 中解析自定义分组 id。
 */
function parseMoreMenuGroupId(key: string) {
  if (!key.startsWith("group:")) return null;

  return key.slice("group:".length);
}

/**
 * 判断字符串是否为范围分组值。
 */
function isRangeGroup(value: unknown): value is ClipboardRange {
  return RANGE_GROUP_OPTIONS.some((option) => {
    return option.value === value;
  });
}

/**
 * 判断字符串是否为分类分组值。
 */
function isCategoryGroup(value: unknown): value is ClipboardCategory {
  return CATEGORY_GROUP_OPTIONS.some((option) => {
    return option.value === value;
  });
}

/**
 * 在可见自定义分组间前后循环；当前未选中分组时，正向取第一个，反向取最后一个。
 */
function selectAdjacentCustomGroup(
  groups: ClipboardGroupRecord[],
  groupId: string | null,
  reverse: boolean,
) {
  if (groups.length === 0) return null;

  const currentIndex = groupId
    ? groups.findIndex((record) => {
        return record.id === groupId;
      })
    : -1;

  if (reverse) {
    if (currentIndex === -1) return groups[groups.length - 1]?.id ?? null;

    return (
      groups[(currentIndex - 1 + groups.length) % groups.length]?.id ?? null
    );
  }

  if (currentIndex === -1) return groups[0]?.id ?? null;

  return groups[(currentIndex + 1) % groups.length]?.id ?? null;
}

/**
 * 判断左右键是否应交给输入控件原生光标导航。
 */
function shouldUseNativeHorizontalNavigation(event: KeyboardEvent) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;

  const tagName = target.tagName.toLowerCase();
  if (target.isContentEditable) return true;

  return tagName === "input" || tagName === "textarea";
}

/**
 * 当前选中分组被删除或不再存在时，回到全部分组。
 */
function ensureSelectedGroupStillExists(groups: ClipboardGroupRecord[]) {
  const selectedGroupId = clipboardViewState.groupId;
  if (!selectedGroupId) return;

  const exists = groups.some((record) => {
    return record.id === selectedGroupId;
  });
  if (exists) return;

  clipboardViewState.groupId = null;
}

export default Group;
