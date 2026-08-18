/**
 * The Library is a single master–detail view: filters, the selected document,
 * and its body mode all live in the URL so a view is shareable/restorable.
 * `type`/`status`/`scope` hold comma-separated multi-selections; `project` is
 * the scope selector (a thoughts-project key); `doc` is the selected doc's
 * absolute-path id and `mode` its body editing mode.
 *
 * Defined here (not in the router) because it is the library feature's view
 * vocabulary; src/router.tsx imports it for the /library route's
 * `validateSearch`.
 */
export type LibrarySearch = {
  type?: string;
  status?: string;
  scope?: string;
  project?: string;
  author?: string;
  tag?: string;
  q?: string;
  doc?: string;
  mode?: "edit" | "preview";
  /** Repo path of the project the user opened the Library from, when they came
   *  via a project's "Docs" affordance, powers the header's back-to-project
   *  button. Absent when the Library is opened standalone (the nav rail). */
  from?: string;
};
