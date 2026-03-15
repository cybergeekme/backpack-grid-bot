import type { Fill, Order, Position, RuntimeState, StrategySnapshot } from '../types';

export class InMemoryStateStore {
  private state: RuntimeState = { workingOrders: [], recentFills: [] };

  get(): RuntimeState {
    return {
      snapshot: this.state.snapshot,
      workingOrders: [...this.state.workingOrders],
      position: this.state.position ? { ...this.state.position } : undefined,
      recentFills: [...this.state.recentFills]
    };
  }

  restore(state: RuntimeState): void {
    this.state = {
      snapshot: state.snapshot,
      workingOrders: [...state.workingOrders],
      position: state.position ? { ...state.position } : undefined,
      recentFills: [...state.recentFills]
    };
  }

  setSnapshot(snapshot: StrategySnapshot): void {
    this.state.snapshot = snapshot;
  }

  setWorkingOrders(orders: Order[]): void {
    this.state.workingOrders = [...orders];
  }

  setPosition(position: Position): void {
    this.state.position = { ...position };
  }

  pushFill(fill: Fill, limit = 100): void {
    this.state.recentFills = [fill, ...this.state.recentFills].slice(0, limit);
  }
}
