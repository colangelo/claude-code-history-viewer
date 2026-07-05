import {
  FeedbackModalContainer,
  FolderSelectorContainer,
  GlobalSearchModalContainer,
  SessionPickerModalContainer,
  ArchiveHubBrowserModalContainer,
} from "@/components/modals";

export const ModalContainer = () => {
  return (
    <>
      <FolderSelectorContainer />
      <FeedbackModalContainer />
      <GlobalSearchModalContainer />
      <SessionPickerModalContainer />
      <ArchiveHubBrowserModalContainer />
    </>
  );
};
