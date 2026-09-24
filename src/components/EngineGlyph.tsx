export function EngineGlyph({
  glyph,
  accentToken,
  className,
}: {
  glyph: string;
  accentToken: string;
  className?: string;
}) {
  return (
    <span className={className} style={{ color: `var(${accentToken})` }} aria-hidden="true">
      {glyph}
    </span>
  );
}
