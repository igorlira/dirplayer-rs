import { Model } from "flexlayout-react";

export const debugLayoutModel = Model.fromJson({
  global: {
    rootOrientationVertical: true,
    tabEnableClose: false,
  },
  layout: {
    type: "row",
    children: [
      {
        type: "tabset",
        enableTabStrip: false,
        minHeight: 44,
        maxHeight: 44,
        children: [
          {
            type: "tab",
            name: "Controls",
            component: "controls",
          },
        ],
      },
      {
        type: "tabset",
        children: [
          {
            type: "tab",
            name: "Call Stack",
            component: "scopes",
          },
        ],
      },
      {
        type: "tabset",
        children: [
          {
            type: "tab",
            name: "Locals",
            component: "locals",
          },
          {
            type: "tab",
            name: "Args",
            component: "args",
          },
          {
            type: "tab",
            name: "Stack",
            component: "stack",
          },
          {
            type: "tab",
            name: "Globals",
            component: "globals",
          },
        ],
      },
    ],
  },
});
