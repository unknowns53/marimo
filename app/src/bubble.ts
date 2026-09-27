export class Bubble {
  constructor(
    private readonly node: HTMLElement,
    onDismiss: () => void,
  ) {
    node.addEventListener("click", onDismiss);
  }

  show(text: string): void {
    this.node.textContent = text;
    this.node.title = "押すと閉じます";
    this.node.classList.add("show");
  }

  hide(): void {
    this.node.classList.remove("show");
  }
}
