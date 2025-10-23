import { Model } from "flexlayout-react";

export const studioLayoutModel = Model.fromJson({
  global: {
    rootOrientationVertical: true,
    tabEnableClose: false,
  },
  borders: [],
  layout: {
    type: "row",
    weight: 100,
    children: [
      {
        type: "tabset",
        minHeight: 50,
        maxHeight: 50,
        children: [
          {
            type: "tab",
            name: "Playback",
            component: "playback",
          },
        ]
      },
      {
        type: "row",
        weight: 70,
        children: [
          {
            type: "tabset",
            weight: 30,
            children: [
              {
                type: "tab",
                name: "Score",
                component: "score",
              },
              {
                type: "tab",
                name: "Cast",
                component: "cast",
              },
            ]
          },
          {
            type: "tabset",
            children: [
              {
                type: "tab",
                name: "Stage",
                component: "stage",
              }
            ]
          },
          {
            type: "tabset",
            weight: 30,
            children: [
              {
                type: "tab",
                name: "Properties",
                component: "properties",
              }
            ]
          },
        ]
      },
      {
        type: "row",
        weight: 30,
        children: [
          {
            type: 'tabset',
            weight: 30,
            children: [
              {
                type: "tab",
                name: "Debug",
                component: "debug",
              },
            ],
          },
          {
            type: 'tabset',
            children: [
              {
                type: "tab",
                name: "Member",
                component: "member",
              }
            ],
          },
        ],
      },
    ]
  }
});
