import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

import { Settings, MessageSquare, Folder } from "lucide-react";

import { useTranslation } from "react-i18next";
import { useModal } from "@/contexts/modal";
import { ThemeMenuGroup } from "./ThemeMenuGroup";
import { LanguageMenuGroup } from "./LanguageMenuGroup";
import { FilterMenuGroup } from "./FilterMenuGroup";
import { FontMenuGroup } from "./FontMenuGroup";
import { AccessibilityMenuGroup } from "./AccessibilityMenuGroup";

export const SettingDropdown = () => {
  const { t } = useTranslation();
  const { openModal } = useModal();

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            id="app-settings-button"
            className="p-2 rounded-lg transition-colors cursor-pointer relative text-muted-foreground/50 hover:text-foreground/80 hover:bg-muted"
            aria-label={t("common.settings.title")}
          >
            <Settings className="w-5 h-5 text-foreground" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>{t('common.settings.title')}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => openModal("folderSelector", { mode: "change" })}
          >
            <Folder className="mr-2 h-4 w-4 text-foreground" />
            <span>{t('common.settings.changeFolder')}</span>
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => openModal("feedback")}>
            <MessageSquare className="mr-2 h-4 w-4 text-foreground" />
            <span>{t("feedback.title")}</span>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <FilterMenuGroup />

          <DropdownMenuSeparator />
          <FontMenuGroup />

          <DropdownMenuSeparator />
          <AccessibilityMenuGroup />

          <DropdownMenuSeparator />
          <ThemeMenuGroup />

          <DropdownMenuSeparator />
          <LanguageMenuGroup />
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
};
