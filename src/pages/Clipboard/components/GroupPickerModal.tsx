import { Modal } from "antd";
import type { FC } from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listClipboardGroups } from "@/commands";
import ClipboardGroupIcon from "@/components/ClipboardGroupIcon";
import type { ClipboardGroupRecord } from "@/types/clipboard";
import { log } from "@/utils/log";

interface GroupPickerModalProps {
  onCancel: () => void;
  onPick: (groupId: string | null) => void;
  open: boolean;
}

/**
 * 批量「移动到分组」的选择弹层：拉取当前分组列表供单选，
 * 含「移出分组」入口。与偏好页的 ClipboardGroupModal（分组定义编辑）不同，
 * 这里只做选择，不提供分组管理。每次打开都重拉分组，保证列表与后端一致。
 */
const GroupPickerModal: FC<GroupPickerModalProps> = (props) => {
  const { t } = useTranslation("clipboard");
  const { open, onCancel, onPick } = props;
  const [groups, setGroups] = useState<ClipboardGroupRecord[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    const loadGroups = async () => {
      setLoading(true);
      try {
        const next = await listClipboardGroups();
        if (!cancelled) setGroups(next);
      } catch (error) {
        log.warn("load groups for picker failed", error);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void loadGroups();

    return () => {
      cancelled = true;
    };
  }, [open]);

  const handlePick = (groupId: string | null) => {
    onPick(groupId);
  };

  return (
    <Modal
      footer={null}
      onCancel={onCancel}
      open={open}
      title={t("batchToolbar.moveGroup")}
    >
      <div className="flex max-h-60 flex-col gap-1 overflow-y-auto">
        {loading ? (
          <div className="py-4 text-center text-ant-secondary text-sm">
            {t("batchToolbar.loadingGroups")}
          </div>
        ) : null}

        {groups.length === 0 && !loading ? (
          <div className="py-4 text-center text-ant-secondary text-sm">
            {t("batchToolbar.noGroups")}
          </div>
        ) : null}

        {groups.map((group) => {
          return (
            <button
              className="flex items-center gap-2 rounded-1 px-2 py-1.5 text-left text-ant-text text-sm transition-colors hover:bg-ant-fill-secondary"
              key={group.id}
              onClick={() => {
                handlePick(group.id);
              }}
              type="button"
            >
              <ClipboardGroupIcon
                className="size-4 shrink-0"
                icon={group.icon}
              />
              <span className="min-w-0 flex-1 truncate">{group.name}</span>
            </button>
          );
        })}

        {groups.length > 0 ? (
          <button
            className="flex items-center gap-2 rounded-1 border-ant-border-secondary border-t px-2 py-1.5 text-left text-ant-secondary text-sm transition-colors hover:bg-ant-fill-secondary"
            onClick={() => {
              handlePick(null);
            }}
            type="button"
          >
            <i
              aria-hidden="true"
              className="i-lucide:folder-minus size-4 shrink-0"
            />
            <span className="min-w-0 flex-1 truncate">
              {t("batchToolbar.removeFromGroup")}
            </span>
          </button>
        ) : null}
      </div>
    </Modal>
  );
};

export default GroupPickerModal;
