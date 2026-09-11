import { Alert, type GetRef, Input, Modal } from "antd";
import type { ChangeEvent, FC } from "react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getClipboardItemEditText, updateClipboardItemText } from "@/commands";
import type { ClipboardItem } from "@/types/clipboard";

interface EditModalProps {
  /**
   * 当前编辑目标；为 null 时关闭。由列表层持有的单例状态注入，引用稳定，
   * 仅在打开新目标时变化（据此重置输入框并拉取编辑源文本）。
   */
  item: ClipboardItem | null;
  /**
   * 关闭弹窗（取消或保存成功后调用）。
   */
  onClose: () => void;
  /**
   * 保存成功回调：Rust 返回的更新后列表条目（含重算的 summary / subKind /
   * availableActions 等派生字段），供列表层整体回填本地镜像。
   */
  onSaved: (updated: ClipboardItem) => void;
}

type TextAreaRef = GetRef<typeof Input.TextArea>;

/**
 * 文本内容编辑弹窗（列表层单例，Ditto 式 edit entry）。
 *
 * 列表视图下 text 条目的 `content` 被 Rust 置空，打开时按 id 拉完整编辑源：
 * 富文本条目（html/rtf）编辑其纯文本表示，保存后由 Rust 转为纯文本并重新识别
 * 子类型（样式丢失是预期，弹窗内有提示）；其余文本条目编辑 `content` 原文。
 * 纯空白内容禁用保存（Rust 侧同样校验兜底）。
 */
const EditModal: FC<EditModalProps> = (props) => {
  const { item, onClose, onSaved } = props;
  const { t } = useTranslation(["clipboard", "common"]);

  const [value, setValue] = useState("");
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const textAreaRef = useRef<TextAreaRef>(null);

  const isRichText = item?.subKind === "html" || item?.subKind === "rtf";
  const isBlank = value.trim().length === 0;

  // item 引用稳定，仅在打开新目标时变化：据此拉取编辑源文本并重置输入框。
  // 拉取失败（窗口关闭竞态等）保持空值，由 Rust 校验与空判兜底。
  useEffect(() => {
    if (!item) return;

    let cancelled = false;
    setLoading(true);

    const load = async () => {
      try {
        const text = await getClipboardItemEditText(item.id);

        if (!cancelled) setValue(text ?? "");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void load();

    return () => {
      cancelled = true;
    };
  }, [item]);

  const handleChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    setValue(event.target.value);
  };

  const handleSave = async () => {
    if (!item) return;

    setSaving(true);

    try {
      const updated = await updateClipboardItemText(item.id, value);

      onSaved(updated);
      onClose();
    } finally {
      setSaving(false);
    }
  };

  /**
   * 弹窗完全打开后聚焦输入框；Windows 下 `autoFocus` 容易早于 Modal 内容稳定挂载。
   */
  const handleAfterOpenChange = (open: boolean) => {
    if (!open) return;

    requestAnimationFrame(() => {
      textAreaRef.current?.focus({ cursor: "end" });
    });
  };

  return (
    <Modal
      afterOpenChange={handleAfterOpenChange}
      confirmLoading={saving}
      destroyOnHidden
      okButtonProps={{ disabled: isBlank || loading }}
      onCancel={onClose}
      onOk={handleSave}
      open={!!item}
      title={t("clipboard:edit.title")}
    >
      {isRichText ? (
        <Alert
          className="mb-2"
          message={t("clipboard:edit.richTextNotice")}
          showIcon
          type="warning"
        />
      ) : null}
      <Input.TextArea
        autoSize={{ maxRows: 12, minRows: 4 }}
        onChange={handleChange}
        placeholder={t("clipboard:edit.placeholder")}
        ref={textAreaRef}
        value={value}
      />
      {isBlank && value.length > 0 ? (
        <div className="mt-1 text-ant-secondary text-xs">
          {t("clipboard:edit.emptyHint")}
        </div>
      ) : null}
    </Modal>
  );
};

export default EditModal;
