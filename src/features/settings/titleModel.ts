export const DEFAULT_TAB_TITLE_MODEL = "claude:haiku";
export const DEFAULT_CODEX_TAB_TITLE_MODEL = "codex:gpt-5.4-mini";

/** The `<select>` value for "no pin": core then follows the default harness. */
export const FOLLOW_HARNESS = "";

export type TitleModelOption = { value: string; label: string };
export type TitleModelGroup = { label: string; options: TitleModelOption[] };

export function defaultTitleModel(defaultHarness: string | null | undefined): string {
  return defaultHarness === "codex" ? DEFAULT_CODEX_TAB_TITLE_MODEL : DEFAULT_TAB_TITLE_MODEL;
}

export function titleModelValue(model: string | null | undefined): string {
  return model?.trim() ? model : FOLLOW_HARNESS;
}

export function titleModelFromValue(value: string): string | null {
  return value === FOLLOW_HARNESS ? null : value;
}

/** Follow first, then the catalogs with Follow's model marked; a pin no catalog lists keeps a group of its own. */
export function titleModelGroups({
  claude,
  codex,
  opencode,
  pinned,
  defaultHarness,
}: {
  claude: TitleModelOption[];
  codex: string[];
  opencode: TitleModelOption[];
  pinned: string | null | undefined;
  defaultHarness: string | null | undefined;
}): TitleModelGroup[] {
  const codexOptions = [
    ...new Set([DEFAULT_CODEX_TAB_TITLE_MODEL.replace(/^codex:/, ""), ...codex]),
  ].map((id) => ({ value: `codex:${id}`, label: id }));
  const resolved = defaultTitleModel(defaultHarness);
  const markDefault = (o: TitleModelOption) =>
    o.value === resolved ? { ...o, label: `${o.label} (default)` } : o;
  const catalogs: TitleModelGroup[] = [
    { label: "Claude", options: claude.map(markDefault) },
    { label: "Codex", options: codexOptions.map(markDefault) },
    { label: "OpenCode", options: opencode.map(markDefault) },
  ].filter((g) => g.options.length > 0);

  const all = catalogs.flatMap((g) => g.options);
  const groups: TitleModelGroup[] = [
    { label: "", options: [{ value: FOLLOW_HARNESS, label: "Follow harness" }] },
    ...catalogs,
  ];
  if (pinned?.trim() && !all.some((o) => o.value === pinned)) {
    groups.push({ label: "Pinned", options: [{ value: pinned, label: pinned }] });
  }
  return groups;
}
