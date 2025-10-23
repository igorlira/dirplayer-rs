import { isElectron } from "./electron";

export function isDebugSession() {
  return window.location.hash.includes("debug") || isElectron();
}

export function isUIShown() {
  return process.env.REACT_APP_SHOW_UI === "true" || isDebugSession();
}
