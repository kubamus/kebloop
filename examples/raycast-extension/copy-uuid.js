import { randomUUID } from "node:crypto";
import { Clipboard, showToast, Toast } from "@raycast/api";

export default async function Command() {
  await Clipboard.copy(randomUUID());
  await showToast({ style: Toast.Style.Success, title: "UUID copied" });
}
