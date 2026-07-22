import type { ExerciseWithMaterial } from "../lib/queries";
import { TONIC_NAMES } from "../lib/format";

/**
 * Material × 12-key coverage (spec AC6, datamodel §4b): rows are materials
 * (ONE row per cell — the 12 keys of a cell share a spec_hash; tonic lives
 * on the fact, the RV grain rule), columns the 12 tonics. An attempted
 * cell shows average accuracy (of graded attempts) and attempt count; a
 * never-drilled cell is BLANK — not 0%. One hot column and 11 blanks is
 * the F5 story ("camped in one key"), told by the data itself.
 */
export function CoverageMatrix({
  exercises,
}: {
  exercises: ExerciseWithMaterial[];
}) {
  interface CellAgg {
    attempts: number;
    graded: number;
    accSum: number;
  }
  interface MaterialRow {
    label: string;
    kind: string;
    byTonic: Map<number, CellAgg>;
    total: number;
  }
  const materials = new Map<string, MaterialRow>();
  for (const ex of exercises) {
    const mat = ex.dim_material;
    if (!mat) continue;
    let row = materials.get(mat.material_id);
    if (!row) {
      row = { label: mat.label, kind: mat.kind, byTonic: new Map(), total: 0 };
      materials.set(mat.material_id, row);
    }
    const agg = row.byTonic.get(ex.tonic) ?? {
      attempts: 0,
      graded: 0,
      accSum: 0,
    };
    agg.attempts += 1;
    if (ex.accuracy !== null) {
      agg.graded += 1;
      agg.accSum += ex.accuracy;
    }
    row.byTonic.set(ex.tonic, agg);
    row.total += 1;
  }

  if (materials.size === 0) {
    return (
      <p className="text-sm text-slate-500">
        No synced exercises yet — nothing to show here, honestly.
      </p>
    );
  }

  const rows = [...materials.values()].sort((a, b) => b.total - a.total);

  return (
    <div className="overflow-x-auto">
      <table className="border-separate border-spacing-1 text-sm">
        <thead>
          <tr>
            <th className="pr-3 text-left font-medium text-slate-500">
              Material
            </th>
            {TONIC_NAMES.map((name) => (
              <th
                key={name}
                scope="col"
                className="w-10 text-center text-xs font-normal text-slate-400"
              >
                {name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.label}>
              <th
                scope="row"
                className="max-w-56 truncate pr-3 text-left font-normal text-slate-900"
                title={row.label}
              >
                {row.label}
              </th>
              {TONIC_NAMES.map((_, tonic) => {
                const agg = row.byTonic.get(tonic);
                if (!agg) {
                  // Never drilled: blank, not 0%.
                  return (
                    <td
                      key={tonic}
                      className="h-8 w-10 rounded bg-slate-50"
                      aria-label="never drilled"
                    />
                  );
                }
                const acc =
                  agg.graded > 0
                    ? `${Math.round((agg.accSum / agg.graded) * 100)}%`
                    : "·";
                return (
                  <td
                    key={tonic}
                    className="h-8 w-10 rounded bg-indigo-100 text-center align-middle text-xs text-indigo-950"
                    title={`${agg.attempts} attempt${agg.attempts === 1 ? "" : "s"}, ${agg.graded} graded`}
                  >
                    {acc}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-2 text-xs text-slate-400">
        Blank = never drilled in that key. &ldquo;·&rdquo; = attempted but never
        graded. The row through all 12 keys is the practice unit — one hot
        column is worth a conversation, not a verdict.
      </p>
    </div>
  );
}
