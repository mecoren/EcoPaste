import type { FC } from "react";
import { useTranslation } from "react-i18next";
import CustomIconButton from "@/components/CustomIconButton";
import Tooltip from "@/components/Tooltip";

interface BatchActionToolbarProps {
  /**
   * 收藏按钮点击；父组件决定目标态（全选未收藏 → 收藏，否则取消）。
   */
  onBatchDelete: () => void;
  onBatchExport: () => void;
  onBatchFavorite: () => void;
  onBatchMerge: () => void;
  onBatchMoveGroup: () => void;
  onBatchPinned: () => void;
  onClearSelection: () => void;
  /**
   * 合并粘贴不可用时的说明（选中集含非文本条目时）；`null` 表示可用或无需解释。
   */
  mergeDisabledReason: string | null;
  selectedCount: number;
}

/**
 * 多选批量操作工具条：多选非空时浮现于列表底部（Footer 上方），
 * 含收藏、置顶、移分组、合并粘贴、导出、删除与清空选中；动作语义与单条快捷键一致，
 * 键位辅助入口已在键盘层（Ctrl+Delete 批量删除等）。
 */
const BatchActionToolbar: FC<BatchActionToolbarProps> = (props) => {
  const { t } = useTranslation("clipboard");
  const {
    selectedCount,
    mergeDisabledReason,
    onBatchFavorite,
    onBatchMerge,
    onBatchPinned,
    onBatchMoveGroup,
    onBatchExport,
    onBatchDelete,
    onClearSelection,
  } = props;

  if (selectedCount === 0) return null;

  const mergeDisabled = mergeDisabledReason !== null || selectedCount < 2;

  return (
    <div className="pointer-events-auto absolute inset-x-3 bottom-2 z-20 flex items-center gap-1 rounded-2 border border-ant-border-secondary bg-ant-container px-2 py-1 shadow-md">
      <span className="min-w-0 flex-1 truncate pl-1 font-medium text-ant-secondary text-xs">
        {t("batchToolbar.selected", { count: selectedCount })}
      </span>

      <Tooltip title={t("batchToolbar.favorite")}>
        <CustomIconButton
          icon={<i aria-hidden="true" className="i-lucide:star" />}
          onClick={onBatchFavorite}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={t("batchToolbar.pinned")}>
        <CustomIconButton
          icon={
            <i aria-hidden="true" className="i-ph:push-pin-bold -rotate-45" />
          }
          onClick={onBatchPinned}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={t("batchToolbar.moveGroup")}>
        <CustomIconButton
          icon={<i aria-hidden="true" className="i-lucide:folder-input" />}
          onClick={onBatchMoveGroup}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={mergeDisabledReason ?? t("batchToolbar.merge")}>
        <CustomIconButton
          disabled={mergeDisabled}
          icon={<i aria-hidden="true" className="i-lucide:combine" />}
          onClick={onBatchMerge}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={t("batchToolbar.export")}>
        <CustomIconButton
          icon={<i aria-hidden="true" className="i-lucide:package-export" />}
          onClick={onBatchExport}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={t("batchToolbar.delete")}>
        <CustomIconButton
          danger
          icon={<i aria-hidden="true" className="i-lucide:trash-2" />}
          onClick={onBatchDelete}
          size="small"
          type="text"
        />
      </Tooltip>

      <Tooltip title={t("batchToolbar.clearSelection")}>
        <CustomIconButton
          icon={<i aria-hidden="true" className="i-lucide:x" />}
          onClick={onClearSelection}
          size="small"
          type="text"
        />
      </Tooltip>
    </div>
  );
};

export default BatchActionToolbar;
