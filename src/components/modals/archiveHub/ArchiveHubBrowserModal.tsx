import { useTranslation } from "react-i18next";
import { Dialog, DialogContent } from "@/components/ui";
import { ArchiveBrowser } from "@/components/ArchiveBrowser";
import type { HubConfig } from "@/services/hubApi";

interface ArchiveHubBrowserModalProps {
  isOpen: boolean;
  onClose: () => void;
  config: HubConfig;
}

export const ArchiveHubBrowserModal = ({
  isOpen,
  onClose,
  config,
}: ArchiveHubBrowserModalProps) => {
  const { t } = useTranslation();

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        className="sm:max-w-6xl h-[85vh] flex flex-col p-4"
        aria-label={t("archiveHub.title")}
      >
        <ArchiveBrowser config={config} />
      </DialogContent>
    </Dialog>
  );
};
