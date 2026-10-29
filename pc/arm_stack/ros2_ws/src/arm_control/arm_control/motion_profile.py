"""为仿真后端生成受限速、同步的关节轨迹。"""

import math
from typing import List


class PrecisionMotionProfile:
    """在满足步长和速度限制的前提下生成平滑运动。"""

    @staticmethod
    def plan(
        start_deg: List[float],
        goal_deg: List[float],
        max_step_deg: float,
        max_velocity_deg_s: float,
        control_period_sec: float,
    ) -> List[List[float]]:
        # 起点和终点长度不一致时，继续执行会导致关节错位，因此直接拒绝。
        if len(start_deg) != len(goal_deg):
            raise ValueError("start and goal lengths must match")

        if not start_deg:
            return []

        # 所有关节使用相同采样数，保证同时开始、同时结束。
        deltas = [goal - start for start, goal in zip(start_deg, goal_deg)]
        max_travel = max(abs(delta) for delta in deltas)
        if max_travel <= 1e-6:
            return []

        step_limited = max(1, math.ceil(max_travel / max(max_step_deg, 1e-6)))
        duration_sec = max(max_travel / max(max_velocity_deg_s, 1e-6), control_period_sec)
        velocity_limited = max(1, math.ceil(duration_sec / control_period_sec))
        steps = max(step_limited, velocity_limited)

        trajectory = []
        for index in range(1, steps + 1):
            # 余弦缓动可以消除起止位置的速度突变。
            ratio = index / steps
            eased = 0.5 - 0.5 * math.cos(math.pi * ratio)
            trajectory.append(
                [start + delta * eased for start, delta in zip(start_deg, deltas)]
            )

        return trajectory

