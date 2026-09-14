import type { FC } from "react";
import { useTranslation } from "react-i18next";
import { updateSettings } from "@/commands";
import type { Settings } from "@/types/settings";
import { cn } from "@/utils/cn";
import type { ControlProps } from "./types";

/** 三档密度预设映射到的现有参数组合；standard 与 Rust `Display::default()` 一致。 */
const DENSITY_PRESETS = {
  comfortable: { fileMaxCount: 5, imageMaxHeight: 100, textMaxLines: 5 },
  compact: { fileMaxCount: 2, imageMaxHeight: 40, textMaxLines: 2 },
  standard: { fileMaxCount: 3, imageMaxHeight: 64, textMaxLines: 3 },
} as const;

type DensityPresetKey = keyof typeof DENSITY_PRESETS;

interface DensityPresetControlProps extends ControlProps {
  settings: Settings;
}

/**
 * 密度预设三档：一键写入 textMaxLines / imageMaxHeight / fileMaxCount 组合。
 * 三项与任何一档完全一致时该档高亮；手动改单项后回到无选中态（custom），
 * 数据驱动无额外状态。
 */
const DensityPresetControl: FC<DensityPresetControlProps> = (props) => {
  const { t } = useTranslation("preferences");
  const { disabled, settings } = props;
  const display = settings.clipboard.display;

  const activePreset = resolveActivePreset(display);

  const handlePresetClick = async (preset: DensityPresetKey) => {
    if (disabled) return;

    await updateSettings({
      clipboard: { display: DENSITY_PRESETS[preset] },
    });
  };

  return (
    <div className="flex items-center gap-1">
      {(Object.keys(DENSITY_PRESETS) as DensityPresetKey[]).map((preset) => {
        return (
          <button
            className={cn(
              "rounded-1 border px-2.5 py-1 text-xs transition-colors motion-reduce:transition-none",
              activePreset === preset
                ? "border-ant-primary bg-ant-primary/10 font-medium text-ant-primary"
                : "border-ant-border text-ant-secondary hover:border-ant-primary hover:text-ant-text",
            )}
            disabled={disabled}
            key={preset}
            onClick={() => {
              void handlePresetClick(preset);
            }}
            type="button"
          >
            {t(`schema.settings.appearance.densityPreset.${preset}`)}
          </button>
        );
      })}
    </div>
  );
};

/** 当前参数组合命中哪一档；不匹配（手动微调过）返回 null。 */
function resolveActivePreset(display: Settings["clipboard"]["display"]) {
  for (const [key, preset] of Object.entries(DENSITY_PRESETS)) {
    const matches =
      display.textMaxLines === preset.textMaxLines &&
      display.imageMaxHeight === preset.imageMaxHeight &&
      display.fileMaxCount === preset.fileMaxCount;

    if (matches) return key as DensityPresetKey;
  }

  return null;
}

export default DensityPresetControl;
