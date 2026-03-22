const VERTEX_SRC = `#version 300 es
in vec2 aPosition;
out vec2 vTexCoord;
void main() {
  vTexCoord = aPosition * 0.5 + 0.5;
  gl_Position = vec4(aPosition, 0.0, 1.0);
}`;

const FRAGMENT_SRC = `#version 300 es
precision mediump float;
uniform sampler2D uData;
uniform int uChannel;
uniform bool uIsDark;
in vec2 vTexCoord;
out vec4 fragColor;

vec3 viridis(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c0 = vec3(0.267, 0.005, 0.329);
  vec3 c1 = vec3(0.282, 0.300, 0.631);
  vec3 c2 = vec3(0.129, 0.572, 0.550);
  vec3 c3 = vec3(0.369, 0.788, 0.384);
  vec3 c4 = vec3(0.993, 0.906, 0.144);
  float s = t * 4.0;
  int i = int(s);
  float f = fract(s);
  if (i == 0) return mix(c0, c1, f);
  if (i == 1) return mix(c1, c2, f);
  if (i == 2) return mix(c2, c3, f);
  return mix(c3, c4, f);
}

vec3 nullColor(float t) {
  return mix(vec3(0.2, 0.4, 0.9), vec3(0.9, 0.2, 0.2), t);
}

void main() {
  vec4 d = texture(uData, vTexCoord);
  float val = uChannel == 0 ? d.r : uChannel == 1 ? d.g : d.b;
  vec3 color = uChannel == 0 ? nullColor(val) : viridis(val);
  float alpha = d.a;
  fragColor = vec4(color, alpha);
}`;

const QUAD_VERTS = new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]);

function compileShader(
  gl: WebGL2RenderingContext | WebGLRenderingContext,
  type: number,
  src: string,
): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, src);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

function linkProgram(
  gl: WebGL2RenderingContext | WebGLRenderingContext,
  vertSrc: string,
  fragSrc: string,
): WebGLProgram | null {
  const vert = compileShader(gl, gl.VERTEX_SHADER, vertSrc);
  const frag = compileShader(gl, gl.FRAGMENT_SHADER, fragSrc);
  if (!vert || !frag) return null;
  const prog = gl.createProgram();
  if (!prog) return null;
  gl.attachShader(prog, vert);
  gl.attachShader(prog, frag);
  gl.linkProgram(prog);
  gl.deleteShader(vert);
  gl.deleteShader(frag);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    gl.deleteProgram(prog);
    return null;
  }
  return prog;
}

export class HeatmapWebGL {
  private gl: WebGL2RenderingContext | WebGLRenderingContext | null;
  private program: WebGLProgram | null;
  private texture: WebGLTexture | null;
  private positionBuffer: WebGLBuffer | null;
  private width: number;
  private height: number;

  constructor(canvas: HTMLCanvasElement) {
    this.width = canvas.width;
    this.height = canvas.height;
    this.program = null;
    this.texture = null;
    this.positionBuffer = null;

    const gl2 = canvas.getContext("webgl2") as WebGL2RenderingContext | null;
    const gl1 = gl2 ?? (canvas.getContext("webgl") as WebGLRenderingContext | null);
    this.gl = gl1;

    if (!this.gl) return;

    const gl = this.gl;
    this.program = linkProgram(gl, VERTEX_SRC, FRAGMENT_SRC);
    if (!this.program) {
      this.gl = null;
      return;
    }

    this.positionBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, this.positionBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, QUAD_VERTS, gl.STATIC_DRAW);

    this.texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  }

  isAvailable(): boolean {
    return this.gl !== null && this.program !== null;
  }

  render(
    data: Uint8Array,
    nRows: number,
    nCols: number,
    channel: number,
    isDark: boolean,
  ): void {
    const gl = this.gl;
    if (!gl || !this.program || !this.texture || !this.positionBuffer) return;

    gl.viewport(0, 0, this.width, this.height);
    gl.clearColor(isDark ? 0.122 : 0.953, isDark ? 0.161 : 0.957, isDark ? 0.231 : 0.965, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    gl.useProgram(this.program);

    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, nCols, nRows, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);

    gl.bindBuffer(gl.ARRAY_BUFFER, this.positionBuffer);
    const posLoc = gl.getAttribLocation(this.program, "aPosition");
    gl.enableVertexAttribArray(posLoc);
    gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);

    const uData = gl.getUniformLocation(this.program, "uData");
    const uChannel = gl.getUniformLocation(this.program, "uChannel");
    const uIsDark = gl.getUniformLocation(this.program, "uIsDark");

    gl.uniform1i(uData, 0);
    gl.uniform1i(uChannel, channel);
    gl.uniform1i(uIsDark, isDark ? 1 : 0);

    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  resize(width: number, height: number): void {
    this.width = width;
    this.height = height;
  }

  dispose(): void {
    const gl = this.gl;
    if (!gl) return;
    if (this.texture) gl.deleteTexture(this.texture);
    if (this.positionBuffer) gl.deleteBuffer(this.positionBuffer);
    if (this.program) gl.deleteProgram(this.program);
    this.texture = null;
    this.positionBuffer = null;
    this.program = null;
    this.gl = null;
  }
}
