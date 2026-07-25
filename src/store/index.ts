import { configureStore } from "@reduxjs/toolkit";
import vmReducer from "./vmSlice";
import uiReducer from "./uiSlice";

const store = configureStore({
  reducer: {
    vm: vmReducer,
    ui: uiReducer,
  },
  // The polyfill/extension builds run inside arbitrary host pages, where
  // window.__REDUX_DEVTOOLS_EXTENSION_COMPOSE__ may be defined by page
  // scripts or other extensions — letting it wrap our store hands control to
  // foreign (possibly broken) code during init. Only hook it up in dev.
  devTools: process.env.NODE_ENV !== 'production',
  middleware: (getDefaultMiddleware) =>
    getDefaultMiddleware({
      serializableCheck: {
        ignoredPaths: ['vm.debugMessages', 'vm.timeoutHandles'],
        ignoredActions: ['vm/debugContentAdded', 'vm/setTimeoutHandle'],
      },
    }),
});
// Infer the `RootState` and `AppDispatch` types from the store itself
export type RootState = ReturnType<typeof store.getState>
// Inferred type: {posts: PostsState, comments: CommentsState, users: UsersState}
export type AppDispatch = typeof store.dispatch

export default store;
