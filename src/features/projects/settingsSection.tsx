import type { ReactNode } from "react";

/**
 * Section chrome for the project-settings dialog.
 *
 * The dialog's sections had been bare divs whose headings sat at `text-sm
 * font-medium` — the same size as the body copy beneath them — separated by a
 * `space-y-3` that also spaced content *within* a section. With no rules, no
 * containers and no size step, nothing marked where one section ended, so the
 * dialog read as a single wall of text while the Scripts block carried far
 * heavier chrome than its peers.
 *
 * Dropping `Card`/`CardTitle` was right (it stopped card-in-dialog border
 * stacking); the mistake was that the separation those borders happened to
 * carry was never replaced. This is the replacement. See ADR-0006.
 */

/** Section heading. 11px uppercase rather than a fifth `text-sm` line: giving
 *  up a size step while gaining case and tracking makes it read as a different
 *  class of thing instead of competing with the prose under it. Same treatment
 *  as the Agents column header (`AgentList.tsx:256`). */
export function SectionHeading({ children }: { children: ReactNode }) {
  return <p className="text-2xs font-semibold uppercase tracking-wider text-ink-dim">{children}</p>;
}

/** One band of the dialog. The rule runs full-bleed against the body's `p-4`
 *  inset (`-mx-4 px-4`) so it reads as a structural divider rather than an
 *  underline of the heading, and the 20px above it plus 20px below is
 *  deliberately double the spacing used inside a section — that contrast is
 *  the grouping signal the flat `space-y-3` never gave. `first` drops both for
 *  the top band, which has the header's own seam above it already. */
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
