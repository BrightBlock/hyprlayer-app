export type WorkshopSection = "agents" | "system" | "skills" | "prompts";

export const WORKSHOP_SECTIONS: readonly WorkshopSection[] = ["agents", "system", "skills", "prompts"];

export type WorkshopSearch = {
  section?: string;
  project?: string;
  folder?: string;
  filter?: string;
  agent?: string;
  tab?: string;
  skill?: string;
  prompt?: string;
};

export function workshopSection(raw: string | undefined): WorkshopSection {
  return WORKSHOP_SECTIONS.find((s) => s === raw) ?? "agents";
}
