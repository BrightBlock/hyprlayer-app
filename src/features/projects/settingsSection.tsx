import type { ReactNode } from "react";


export function SectionHeading({ children }: { children: ReactNode }) {
  return <p className="text-label text-ink-dim">{children}</p>;
}

export function SettingsSection({ first, children }: { first?: boolean; children: ReactNode }) {
  return (
    <section
      className={
        first ? "shrink-0" : "-mx-4 mt-5 shrink-0 border-t border-edge/60 px-4 pt-5"
      }
    >
      {children}
    </section>
  );
}
