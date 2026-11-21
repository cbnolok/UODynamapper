"""Backward warp by a flow field in pixels (the Practical-RIFE warp, written here)."""
import torch
import torch.nn.functional as F


def warp(ten_input, ten_flow):
    b, _, h, w = ten_flow.shape
    hor = torch.linspace(-1.0, 1.0, w).view(1, 1, 1, w).expand(b, -1, h, -1)
    ver = torch.linspace(-1.0, 1.0, h).view(1, 1, h, 1).expand(b, -1, -1, w)
    grid = torch.cat([hor, ver], 1)
    flow = torch.cat([ten_flow[:, 0:1] / ((w - 1.0) / 2.0), ten_flow[:, 1:2] / ((h - 1.0) / 2.0)], 1)
    g = (grid + flow).permute(0, 2, 3, 1)
    return F.grid_sample(ten_input, g, mode='bilinear', padding_mode='border', align_corners=True)
