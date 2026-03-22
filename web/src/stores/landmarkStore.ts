import { create } from "zustand";

export type LandmarkType = "null_surge" | "outlier" | "boundary";

export type Landmark = {
  rowOffset: number;
  rowCount: number;
  type: LandmarkType;
  severity: number;
  affectedCols: number[];
};

type LandmarkStore = {
  landmarks: Landmark[];
  setLandmarks: (landmarks: Landmark[]) => void;
};

export const useLandmarkStore = create<LandmarkStore>((set) => ({
  landmarks: [],
  setLandmarks: (landmarks) => set({ landmarks }),
}));
