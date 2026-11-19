import { matchesFilter, type FilterFieldSpec, type FilterNode } from "./filterTree";
import type { Message } from "./types";

export const MESSAGE_FILTER_SPECS: FilterFieldSpec[] = [
  { key: "name", label: "Name" },
  { key: "id", label: "ID" },
  { key: "kind", label: "Kind" },
  { key: "data", label: "Data" },
];

function fieldValue(row: Message, field: string): string {
  switch (field) {
    case "name":
      return row.name;
    case "id":
      return row.addr;
    case "kind":
      return row.kind;
    case "data":
      return row.data;
    default:
      return "";
  }
}

export function matchesMessageFilter(
  filter: FilterNode | null,
  row: Message,
): boolean {
  return matchesFilter(filter, (field) => fieldValue(row, field));
}
