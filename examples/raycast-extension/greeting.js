import { Clipboard, getPreferenceValues, showToast, Toast } from "@raycast/api";

export default async function Command() {
  const { name } = getPreferenceValues();
  await Clipboard.copy(`Hello, ${name}!`);
  await showToast({ style: Toast.Style.Success, title: "Greeting copied" });
}
