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
  from?: string;
};
