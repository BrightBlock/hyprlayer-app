import { useNavigate, useSearch } from "@tanstack/react-router";

import { ServersSettings } from "@/features/hosts/ServersSettings";
import { parseSettingsSection } from "@/features/settings/sections";
import { SettingsView } from "@/features/settings/SettingsView";

export function Settings() {
  const { section, project } = useSearch({ from: "/settings" });
  const navigate = useNavigate({ from: "/settings" });
  return (
    <SettingsView
      section={parseSettingsSection(section)}
      // replace, so one Escape still leaves Settings however many sections were visited
      onSection={(next) =>
        void navigate({ search: { section: next === "general" ? undefined : next, project }, replace: true })
      }
      project={project}
      servers={<ServersSettings />}
    />
  );
}
