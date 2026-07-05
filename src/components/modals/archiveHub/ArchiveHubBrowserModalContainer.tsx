import { ArchiveHubBrowserModal } from "./ArchiveHubBrowserModal";
import { useModal } from "@/contexts/modal";
import { useAppStore } from "@/store/useAppStore";

export const ArchiveHubBrowserModalContainer: React.FC = () => {
    const { isOpen, closeModal } = useModal();
    const { archiveHubUrl, archiveHubToken } = useAppStore(
        (state) => state.userMetadata.settings
    );

    if (!isOpen("archiveHubBrowser")) return null;
    if (!archiveHubUrl || !archiveHubToken) return null;

    return (
        <ArchiveHubBrowserModal
            isOpen={true}
            onClose={() => closeModal("archiveHubBrowser")}
            config={{ url: archiveHubUrl, token: archiveHubToken }}
        />
    );
};
