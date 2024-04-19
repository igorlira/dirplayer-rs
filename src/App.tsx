import './App.css';
import VMProvider from './components/VMProvider'
import store from './store';
import { isDebugSession } from './utils/debug';
import DirStudio from './views/DirStudio/DirStudio';
import { Provider as StoreProvider } from 'react-redux'

function App() {
  const showDebugUi = isDebugSession();
  return (
    <div className="App">
      <StoreProvider store={store}>
        <VMProvider>
          <DirStudio showDebugUi={showDebugUi} autoPlay={!showDebugUi} />
        </VMProvider>
      </StoreProvider>
    </div>
  );
}

export default App;
