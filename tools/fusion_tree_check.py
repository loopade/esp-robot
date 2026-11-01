"""Fusion 360 只读装配关系检查脚本。

运行后会在当前用户桌面生成 fusion_tree_report.json，并在 Fusion 中弹出摘要。
脚本不会修改设计、关节或组件。
"""

import json
import os
import traceback

import adsk.core
import adsk.fusion


def _name(occurrence):
    """优先使用实例完整路径，避免同名组件被错误合并。"""
    if occurrence is None:
        return "<无组件>"
    try:
        return occurrence.fullPathName
    except Exception:
        return occurrence.name


def _joint_type(joint):
    """读取 Fusion 关节类型；读取失败时保留原始可诊断信息。"""
    try:
        return str(joint.jointMotion.jointType)
    except Exception:
        return "读取失败"


def _find_cycles(edges):
    """使用深度优先搜索检查 parent -> child 图中是否存在闭环。"""
    graph = {}
    for edge in edges:
        graph.setdefault(edge["parent"], []).append(edge["child"])

    visiting = set()
    visited = set()
    cycles = []

    def visit(node, path):
        if node in visiting:
            start = path.index(node) if node in path else 0
            cycles.append(path[start:] + [node])
            return
        if node in visited:
            return
        visiting.add(node)
        for child in graph.get(node, []):
            visit(child, path + [child])
        visiting.remove(node)
        visited.add(node)

    for node in graph:
        visit(node, [node])
    return cycles


def _origin_info(joint, side):
    """读取关节端点的可诊断信息，不修改 Fusion 设计。"""
    try:
        value = getattr(joint, side)
        origin = getattr(value, "origin", None)
        if origin is None:
            origin = getattr(getattr(value, "geometry", None), "origin", None)
        if origin is None:
            return {"available": False, "value_type": type(value).__name__}
        return {
            "available": True,
            "value_type": type(value).__name__,
            "origin": list(origin.asArray()),
        }
    except Exception as exc:
        return {"available": False, "error": str(exc)}


def _transform_translation(occurrence):
    """读取 occurrence 的装配变换平移量，单位保持 Fusion 默认厘米。"""
    try:
        return list(occurrence.transform.translation.asArray())
    except Exception as exc:
        return {"error": str(exc)}


def _transform_matrix(occurrence):
    """读取 occurrence 的 4x4 装配变换矩阵。"""
    try:
        return list(occurrence.transform.asArray())
    except Exception as exc:
        return {"error": str(exc)}


def _collect_component_joints(root):
    """递归读取根组件及嵌套组件中的 Joint 和 As-Built Joint。"""
    joints = []
    seen = set()
    components = [root]
    visited_components = set()

    while components:
        component = components.pop()
        token = getattr(component, "entityToken", None) or component.name
        if token in visited_components:
            continue
        visited_components.add(token)

        for collection_name in ("joints", "asBuiltJoints"):
            try:
                collection = getattr(component, collection_name)
                for joint in collection:
                    joint_token = getattr(joint, "entityToken", None) or str(id(joint))
                    if joint_token not in seen:
                        seen.add(joint_token)
                        joints.append((joint, component))
            except Exception:
                pass

        try:
            for occurrence in component.occurrences:
                components.append(occurrence.component)
        except Exception:
            pass

    return joints


def run(context):
    app = adsk.core.Application.get()
    ui = app.userInterface
    report = {
        "document": None,
        "root_component": None,
        "grounded_occurrences": [],
        "grounded_to_parent_occurrences": [],
        "joints": [],
        "parent_count": {},
        "cycles": [],
        "is_tree_candidate": False,
    }

    try:
        product = app.activeProduct
        design = adsk.fusion.Design.cast(product)
        if design is None:
            raise RuntimeError("当前文档不是 Fusion 设计。")

        root = design.rootComponent
        report["document"] = app.activeDocument.name
        report["root_component"] = root.name

        # 递归读取所有组件，避免漏掉嵌套组件内部的刚性和旋转关节。
        joints = _collect_component_joints(root)

        edges = []
        for joint, owner_component in joints:
            # 插件源码使用 occurrenceTwo 作为 parent，occurrenceOne 作为 child。
            parent = _name(joint.occurrenceTwo)
            child = _name(joint.occurrenceOne)
            edge = {
                "joint": joint.name,
                "type": _joint_type(joint),
                "parent_component2": parent,
                "child_component1": child,
                "owner_component": getattr(owner_component, "name", "<unknown>"),
                "joint_class": type(joint).__name__,
                "origin_one": _origin_info(joint, "geometryOrOriginOne"),
                "origin_two": _origin_info(joint, "geometryOrOriginTwo"),
                "occurrence_one_transform": _transform_translation(joint.occurrenceOne),
                "occurrence_two_transform": _transform_translation(joint.occurrenceTwo),
                "occurrence_one_matrix": _transform_matrix(joint.occurrenceOne),
                "occurrence_two_matrix": _transform_matrix(joint.occurrenceTwo),
            }
            report["joints"].append(edge)
            edges.append({"parent": parent, "child": child})
            report["parent_count"][child] = report["parent_count"].get(child, 0) + 1

        # 运动链接是关节之间的附加约束，不是 parent -> child 边，单独记录。
        report["motion_links"] = []
        try:
            for motion_link in root.motionLinks:
                report["motion_links"].append(motion_link.name)
        except Exception:
            pass

        # 记录所有层级中的接地和“固定到父对象”实例，避免漏掉嵌套组件。
        try:
            all_occurrences = root.allOccurrences
        except Exception:
            all_occurrences = root.occurrences
        for occurrence in all_occurrences:
            try:
                if occurrence.isGrounded:
                    report["grounded_occurrences"].append(_name(occurrence))
            except Exception:
                pass
            try:
                if occurrence.isGroundedToParent:
                    report["grounded_to_parent_occurrences"].append(_name(occurrence))
            except Exception:
                pass

        report["cycles"] = _find_cycles(edges)
        child_names = {edge["child"] for edge in edges}
        parent_names = {edge["parent"] for edge in edges}
        report["root_candidates"] = sorted(parent_names - child_names)
        report["base_link_reachable"] = False
        if report["grounded_occurrences"]:
            start = report["grounded_occurrences"][0]
            graph = {}
            for edge in edges:
                graph.setdefault(edge["parent"], []).append(edge["child"])
            reachable = set()
            pending = [start]
            while pending:
                node = pending.pop()
                if node in reachable:
                    continue
                reachable.add(node)
                pending.extend(graph.get(node, []))
            report["base_link_reachable"] = all(
                node in reachable for node in parent_names | child_names
            )
        duplicate_parents = [
            name for name, count in report["parent_count"].items() if count > 1
        ]
        report["multiple_parent_children"] = duplicate_parents
        report["is_tree_candidate"] = bool(
            len(report["grounded_occurrences"]) == 1
            and not report["cycles"]
            and not duplicate_parents
            and len(report["root_candidates"]) == 1
            and report["base_link_reachable"]
        )

        # 生成可直接核对的逐条关系文本，避免只看摘要而缺少证据。
        relation_lines = [
            f"{index + 1}. {edge['joint']}: {edge['parent_component2']} -> {edge['child_component1']} ({edge['type']})"
            for index, edge in enumerate(report["joints"])
        ]
        report["relation_lines"] = relation_lines
        diagnostic_lines = [
            (
                f"{edge['joint']}: 所属组件={edge['owner_component']}，"
                f"API类型={edge['joint_class']}，"
                f"端点1={'有' if edge['origin_one'].get('available') else '无'}，"
                f"端点2={'有' if edge['origin_two'].get('available') else '无'}，"
                f"child坐标={edge['occurrence_one_transform']}，"
                f"parent坐标={edge['occurrence_two_transform']}"
            )
            for edge in report["joints"]
        ]
        report["diagnostic_lines"] = diagnostic_lines

        desktop = os.path.join(os.path.expanduser("~"), "Desktop")
        output_path = os.path.join(desktop, "fusion_tree_report.json")
        with open(output_path, "w", encoding="utf-8") as output:
            json.dump(report, output, ensure_ascii=False, indent=2)

        text_path = os.path.join(desktop, "fusion_tree_report.txt")
        with open(text_path, "w", encoding="utf-8-sig") as output:
            output.write("Fusion 装配关系逐条报告\n\n")
            output.write("\n".join(relation_lines))
            output.write("\n\n根节点候选：\n")
            output.write("\n".join(report["root_candidates"]))

        summary = (
            f"关节数量：{len(report['joints'])}\n"
            f"接地组件数量：{len(report['grounded_occurrences'])}\n"
            f"固定到父对象数量：{len(report['grounded_to_parent_occurrences'])}\n"
            f"运动链接数量：{len(report['motion_links'])}\n"
            f"闭环数量：{len(report['cycles'])}\n"
            f"多父节点数量：{len(duplicate_parents)}\n"
            f"根节点候选数量：{len(report['root_candidates'])}\n"
            f"所有组件均从 base_link 可达：{'是' if report['base_link_reachable'] else '否'}\n"
            f"树结构候选：{'是' if report['is_tree_candidate'] else '否'}\n\n"
            f"逐条关系：\n" + "\n".join(relation_lines) + "\n\n"
            f"关节读取诊断：\n" + "\n".join(diagnostic_lines) + "\n\n"
            f"JSON 报告：{output_path}\n"
            f"文本报告：{text_path}"
        )
        ui.messageBox(summary, "Fusion 装配关系检查")
    except Exception:
        ui.messageBox(traceback.format_exc(), "Fusion 装配关系检查失败")
