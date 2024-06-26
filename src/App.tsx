import './App.css';
import VMProvider from './components/VMProvider'
import store from './store';
import { isUIShown } from './utils/debug';
import DirStudio from './views/DirStudio/DirStudio';
import { Provider as StoreProvider } from 'react-redux'

function App() {
  const showDebugUi = isUIShown();
  return (
    <div className="App">
      <StoreProvider store={store}>
        <VMProvider>
          <DirStudio showDebugUi={showDebugUi} />
        </VMProvider>
      </StoreProvider>
    </div>
  );
}

export default App;
