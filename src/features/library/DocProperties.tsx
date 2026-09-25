import type { DocContent } from "@/lib/bindings";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES } from "./constants";
import { fieldValue, placeholder as placeholderFor, type FmDraft, type SchemaKey } from "./fmDraft";
import { AuthorField, DateField, EnumSelect, PropertyRow, TagsField, TextField, TicketField } from "./propertyControls";
import { RawFrontmatter } from "./RawFrontmatter";
import { RelatedField } from "./RelatedField";

type Props = {
  doc: DocContent;
  draft: FmDraft;
  onChange: (key: SchemaKey, next: string | string[]) => void;
  raw: string | null;
  setRaw: (text: string | null) => void;
  disabled: boolean;
};

// Rows follow fmDraft's SCHEMA_KEYS order: title, type, date, status, ticket, project, scope, tags, author, related.
export function DocProperties({ doc, draft, onChange, raw, setRaw, disabled }: Props) {
  const fm = doc.meta.fm;
  const fmKeys = doc.fmKeys;

  const str = (key: SchemaKey) => fieldValue(fm, fmKeys, draft, key) as string;
  const list = (key: SchemaKey) => fieldValue(fm, fmKeys, draft, key) as string[];
  const hint = (key: SchemaKey) => placeholderFor(fm, fmKeys, key);

  return (
    <div>
      {!doc.fmError && raw === null && (
        <fieldset disabled={disabled} className="space-y-3">
          <PropertyRow id="fm-title" label="Title">
            <TextField id="fm-title" value={str("title")} placeholder={hint("title")} onChange={(v) => onChange("title", v)} />
          </PropertyRow>
          <PropertyRow id="fm-type" label="Type">
            <EnumSelect
              id="fm-type"
              value={str("type")}
              canonical={DOC_TYPES}
              placeholder={hint("type")}
              onChange={(v) => onChange("type", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-date" label="Date">
            <DateField id="fm-date" value={str("date")} placeholder={hint("date")} onChange={(v) => onChange("date", v)} />
          </PropertyRow>
          <PropertyRow id="fm-status" label="Status">
            <EnumSelect
              id="fm-status"
              value={str("status")}
              canonical={DOC_STATUSES}
              placeholder={hint("status")}
              onChange={(v) => onChange("status", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-ticket" label="Ticket">
            <TicketField
              id="fm-ticket"
              value={str("ticket")}
              placeholder={hint("ticket")}
              onChange={(v) => onChange("ticket", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-project" label="Project">
            <TextField
              id="fm-project"
              value={str("project")}
              placeholder={hint("project")}
              onChange={(v) => onChange("project", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-scope" label="Scope">
            <EnumSelect
              id="fm-scope"
              value={str("scope")}
              canonical={DOC_SCOPES}
              placeholder={hint("scope")}
              onChange={(v) => onChange("scope", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-tags" label="Tags">
            <TagsField id="fm-tags" value={list("tags")} onChange={(v) => onChange("tags", v)} />
          </PropertyRow>
          <PropertyRow id="fm-author" label="Author">
            <AuthorField
              id="fm-author"
              value={str("author")}
              placeholder={hint("author")}
              onChange={(v) => onChange("author", v)}
            />
          </PropertyRow>
          <PropertyRow id="fm-related" label="Related">
            <RelatedField id={doc.meta.id} values={list("related")} onChange={(v) => onChange("related", v)} />
          </PropertyRow>
        </fieldset>
      )}
      <RawFrontmatter
        doc={doc}
        raw={raw}
        setRaw={setRaw}
        draftNonEmpty={Object.keys(draft).length > 0}
        disabled={disabled}
      />
    </div>
  );
}
