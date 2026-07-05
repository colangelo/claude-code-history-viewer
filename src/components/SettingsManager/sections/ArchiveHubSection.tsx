/**
 * Settings section for the archive hub connection (URL + bearer token).
 * Values persist via UserSettings (archiveHubUrl / archiveHubToken).
 */

export interface ArchiveHubSectionProps {
  initialUrl?: string;
  initialToken?: string;
  onSave: (url: string, token: string) => void | Promise<void>;
}

export function ArchiveHubSection(_props: ArchiveHubSectionProps) {
  void _props;
  return <div data-testid="archive-hub-section" />;
}
