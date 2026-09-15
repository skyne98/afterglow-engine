/** Invert the zoom, rotation, and mirror around the canvas center. */
export function inversePaintView(
  x: number, y: number, zoom: number, rotationDegrees: number, mirror: boolean,
): [number, number] {
  const angle = rotationDegrees * Math.PI / 180;
  const cosine = Math.cos(angle), sine = Math.sin(angle);
  return [
    (cosine * x + sine * y) / zoom * (mirror ? -1 : 1),
    (-sine * x + cosine * y) / zoom,
  ];
}
