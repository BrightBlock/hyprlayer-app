import type { ReactNode } from "react";

import { useBootstrap, useHosts } from "@/lib/queries";
import { PageError, PageSpinner } from "@/components/ui";

import { AdvancedSettings } from "./AdvancedSettings";
import { AgentControlSettings } from "./AgentControlSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { BinariesSettings } from "./BinariesSettings";
import { GeneralSettings } from "./GeneralSettings";
import { GitSettings } from "./GitSettings";
import { GoalsSettings } from "./GoalsSettings";
import { HarnessesSection } from "./harnesses/HarnessesSection";
import { KeyboardSettings } from "./KeyboardSettings";
import { NotificationsSettings } from "./NotificationsSettings";
import type { SettingsSection } from "./sections";
import { SettingsNav } from "./SettingsNav";

/** The full Settings view. Servers comes from the hosts feature as a slot. */
export function SettingsView({
  section,
  onSection,
  servers,
}: {
  section: SettingsSection;
  onSection: (section: SettingsSection) => void;
  servers: ReactNode;
}) {
  const bootstrap = useBootstrap();
  // This machine plus the saved remotes.
  const serverCount = (useHosts().data?.hosts?.length ?? 0) + 1;

  return (
    <div className="flex h-full min-h-0">
      <SettingsNav section={section} onSection={onSection} serverCount={serverCount} />
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[720px] space-y-3.5 px-8 py-6">
          <SectionBody section={section} servers={servers} bootstrap={bootstrap} />
        </div>
      </div>
    </div>
  );
}

function SectionBody({
  section,
  servers,
  bootstrap,
}: {
  section: SettingsSection;
  servers: ReactNode;
  bootstrap: ReturnType<typeof useBootstrap>;
}) {
  // Servers and appearance are desktop-local, so they work without a server connection.
  if (section === "servers") return servers;
  if (section === "appearance") return <AppearanceSettings />;
  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;
  const data = bootstrap.data!;
  switch (section) {
    case "general":
      return <GeneralSettings data={data} />;
    case "harnesses":
      return <HarnessesSection settings={data.settings} binaries={data.binaries} />;
    case "git":
      return <GitSettings settings={data.settings} />;
    case "notifications":
      return <NotificationsSettings settings={data.settings} />;
    case "goals":
      return <GoalsSettings settings={data.settings} />;
    case "agentControl":
      return <AgentControlSettings settings={data.settings} />;
    case "keyboard":
      return <KeyboardSettings settings={data.settings} />;
    case "advanced":
      return <AdvancedSettings settings={data.settings} />;
    case "binaries":
      return <BinariesSettings binaries={data.binaries} />;
  }
}
