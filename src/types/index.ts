// 跨页共享类型集中导出：与后端 Command 签名一一对应（见 Architecture-frontend.md）。
export interface ServerStatus {
  running: boolean;
  host: string | null;
  port: number | null;
}
