interface Vars {
  store: string;
  lock: string;
  link: Record<string, string>;
  pc: string;
  storeJunk: number;
  linkJunk: number;
}

const links = (v: Vars) => Object.values(v.link);

export const title =
  "A skill's store, links and lock through installs, updates, removes and crashes";

export const state = (v: Vars) => {
  const busy = v.pc === "idle" ? "" : " (operation running)";
  const junk = v.storeJunk + v.linkJunk > 0 ? ", leftovers" : "";
  if (
    v.lock === "none" &&
    v.store === "none" &&
    links(v).every((l) => l === "none" || l === "foreign")
  ) {
    return `Not installed${junk}${busy}`;
  }
  if (v.lock === "none") return `Half installed, no lock${junk}${busy}`;
  if (v.store !== v.lock)
    return `Installed, store differs from lock${junk}${busy}`;
  return `Installed${junk}${busy}`;
};

export const group = (v: Vars) => (v.pc === "idle" ? "idle" : "busy");

export const colors = { idle: "blue", busy: "orange" };

const labels: Record<string, string> = {
  InstallStart: "you install, sweeping store leftovers first",
  InstallClear: "the install clears the store and copies",
  InstallStore: "the store is moved in",
  InstallLink: "a link is made",
  InstallLock: "the lock is written",
  InstallUndo: "a step fails and the install is undone",
  ReplaceStart: "you update, sweeping store leftovers first",
  ReplaceSweep: "the update sweeps link leftovers",
  ReplaceAside: "the old store is set aside",
  ReplaceStore: "the new store is moved in",
  ReplaceLink: "a link is looked at",
  ReplaceMake: "a link is made",
  ReplaceRefreshAside: "a copied link is set aside",
  ReplaceRefreshIn: "a fresh copy is moved in",
  ReplaceLock: "the lock is written",
  ReplaceClean: "the old store and set-asides are removed",
  ReplaceUndo: "a step fails and the update is undone",
  RemoveStart: "you remove, sweeping leftovers first",
  RemoveLinks: "an owned link is removed",
  RemoveStore: "the store is removed",
  RemoveLock: "the lock entry is removed",
  RemoveFails: "a step of the remove fails",
  RestoreStart: "you restore, sweeping leftovers first",
  RestoreAside: "the stale store is set aside",
  RestoreAdopt: "the store is fetched back at the locked commit",
  RestoreClean: "the set-aside store is removed",
  RestoreRelink: "a link is repaired",
  RestoreFails: "a step of the restore fails",
  Crash: "the server is killed",
  Damage: "someone else changes the files",
  Done: "nothing more happens",
};

export const step = (action: string) => labels[action] ?? action;
