export function getBasePath(fullPath: string) {
  const pathComponents = fullPath.split("/");
  if (pathComponents.length > 0) {
    pathComponents.pop();
  }

  return pathComponents.join("/");
}

export function getFullPathFromOrigin(path: string) {
  if (path.includes('://')) {
    return path;
  }

  const currentPathSegments = window.location.pathname.split('/');
  if (currentPathSegments.length > 0 && currentPathSegments[currentPathSegments.length - 1] !== '') {
    currentPathSegments.pop();
    currentPathSegments.push('');
  }
  const currentPathWithSlash = currentPathSegments.join('/');
  return window.location.origin + currentPathWithSlash + path;
}
