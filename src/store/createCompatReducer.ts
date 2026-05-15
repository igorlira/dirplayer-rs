type ActionCreatorWithType = {
  type: string;
  (...args: any[]): { type: string };
};

type AnyAction = {
  type: string;
};

type CaseReducer<State, Action extends AnyAction> = (state: State, action: Action) => State;

// Legacy pages patch global built-ins in ways that break RTK's runtime reducer
// builders (`createSlice` and `createReducer`) during store setup.
// This keeps the typed `builder.addCase(...)` authoring style without depending on
// those runtime paths.
class CompatReducerBuilder<State> {
  private handlers: Record<string, CaseReducer<State, AnyAction>> = {};

  addCase<ActionCreator extends ActionCreatorWithType>(
    actionCreator: ActionCreator,
    reducer: CaseReducer<State, ReturnType<ActionCreator>>,
  ): this {
    this.handlers[actionCreator.type] = reducer as CaseReducer<State, AnyAction>;
    return this;
  }

  build(initialState: State) {
    return (state: State = initialState, action: AnyAction): State => {
      const handler = this.handlers[action.type];
      return handler ? handler(state, action) : state;
    };
  }
}

export function createCompatReducer<State>(
  initialState: State,
  buildCases: (builder: CompatReducerBuilder<State>) => void,
) {
  const builder = new CompatReducerBuilder<State>();
  buildCases(builder);
  return builder.build(initialState);
}
