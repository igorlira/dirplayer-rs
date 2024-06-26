export function isDebugSession() {
  return window.location.hash.includes("debug");
}

export function isUIShown() {
  return process.env.REACT_APP_SHOW_UI === "true" || isDebugSession();
}
