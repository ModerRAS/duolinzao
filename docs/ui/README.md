# 界面参考

这些图是当前的视觉参考，不是像素级终稿。产品规则以 [一期需求](../requirements-phase1.md) 为准。图和正文冲突时，按正文实现，并看下面的「不要照搬」。

食堂端以**左侧栏**为主，菜、饭、汤分开。评语、截止后改单两张是更早的稿，导航彼此不一致，**不作为主导航**。内容可以参考，外壳跟主参考的左侧栏走。

## 员工端竖屏

| 屏幕 | 参考图 |
|------|--------|
| 首页：尚未选择班制，默认不报餐 | [employee-home.jpg](employee-home.jpg) |
| 选择班制 | [employee-shift.jpg](employee-shift.jpg) |
| 本周报餐 | [employee-week.jpg](employee-week.jpg) |
| 评分 | [employee-rating.jpg](employee-rating.jpg) |
| 模板预览：绿色报餐，橙色不报 | [employee-template-preview.jpg](employee-template-preview.jpg) |
| 邀请码注册 | [employee-invite-register.jpg](employee-invite-register.jpg) |
| 用描述生成模板 | [employee-template-generate.jpg](employee-template-generate.jpg) |
| 绑定食堂 | [employee-bind-canteens.jpg](employee-bind-canteens.jpg) |
| 设备与 Passkey（含新设备页内确认） | [employee-devices-passkey.jpg](employee-devices-passkey.jpg) |

## 食堂端横屏（主参考）

左侧栏大致是：人数总览、菜谱、选菜优化、强提醒、评分、评语、改单。菜、饭、汤用页签或分列，不混成一张列表。

| 屏幕 | 参考图 |
|------|--------|
| 就餐人数总览 | [canteen-headcount.jpg](canteen-headcount.jpg) |
| 菜谱列表 | [canteen-menu.jpg](canteen-menu.jpg) |
| 评分汇总 | [canteen-ratings.jpg](canteen-ratings.jpg) |
| 选菜优化：周视图里按餐选菜、饭、汤 | [canteen-optimizer.jpg](canteen-optimizer.jpg) |
| 登录（无左侧栏） | [canteen-login.jpg](canteen-login.jpg) |
| 设备与多账号 | [canteen-devices.jpg](canteen-devices.jpg) |
| 菜品表单：新建为空表，编辑带出已有内容 | [canteen-edit-dish.jpg](canteen-edit-dish.jpg) |
| 邀请码一次性展示 | [canteen-invite-show.jpg](canteen-invite-show.jpg) |
| 强提醒 | [canteen-alert.jpg](canteen-alert.jpg) |
| 数据导出 | [canteen-export.jpg](canteen-export.jpg) |
| 审计日志 | [canteen-audit.jpg](canteen-audit.jpg) |
| 邮箱设置 | [canteen-email-settings.jpg](canteen-email-settings.jpg) |
| 强提醒阈值 | [canteen-alert-threshold.jpg](canteen-alert-threshold.jpg) |
| 账号找回 | [canteen-account-recovery.jpg](canteen-account-recovery.jpg) |
| LLM 渠道配置 | [canteen-llm-channels.jpg](canteen-llm-channels.jpg) |

选菜优化这张要看的是：菜、饭、汤分列，卡片带图片和口味标签（如酸、甜、开胃），勾选自动保存，并有「保存并下一餐」。颜色严格表示评分偏好：更高更绿，更低更红。标签不代替这个色阶，也不代替组内排序。

后六张是总管理页面，仍用左侧栏。侧栏只显示该账号有权限的一期画面。

## 更早的稿（导航不作为主参考）

| 屏幕 | 参考图 | 说明 |
|------|--------|------|
| 评语 | [canteen-comments.jpg](canteen-comments.jpg) | 侧栏和主参考不完全一样。看匿名评语卡片即可 |
| 截止后改单 | [canteen-override.jpg](canteen-override.jpg) | 侧栏里有订单、餐补等一期没有的入口。看改单表格即可 |

## 不要照搬

- 班制图上的「长白班五休二」不实现。只保留「上二休四」和「自定义」。「用描述生成模板」是模板生成器，不是第三种班制。
- 本周报餐图里，没报的餐画成空心按钮，底部还有「保存」。锁定规则仍是绿色表示报餐、橙色表示不报，切换写回个人 DSL。空心样式和这颗保存按钮不覆盖该规则。绿 / 橙以模板预览图为准。
- 评分图把菜、汤、饭排在同一列表，评语画成整餐一条。实现仍把菜、饭、汤分开，评语仍挂在每一道餐品自己的评分上。图上的图片和 −2～2 要保留。
- 菜谱列表和评分汇总的图没有画出图片列。列表可以显示缩略图，不强制。没有图片时用中性占位，不用别的菜品图充数。评分汇总和选菜仍显示图片或占位。
- 菜品表单只有一张终稿，就是编辑菜这张布局。新建是同一页的空表，默认启用。没有备注字段。类型只能是菜、饭、汤。口味标签由食堂在这张表单里维护。
- 选菜优化图上的「预计评分：87分」「评分偏好：高」和「2 大人 + 2 儿童」不实现。色阶只表示偏好高低（更高更绿，更低更红），界面不出现偏好数字、倾向、权重或 Elo。
- 人数总览图下半的分类占比，以及同一天里「午餐已锁定、晚餐未锁定」，不实现。人数仍是有效报餐人数。锁定仍是一次锁定次日三餐。
- 强提醒图用左侧栏，不再用顶栏「预警中心」。图上的均分是 3 分一带，细类写成「川菜 / 热菜」。一期分数仍是 −2～2，类别仍是菜、饭、汤。不自动下架，停用要去菜品表单确认。
- 旧稿里的顶栏、第二套侧栏、订单管理和餐补规则都不要做成导航。
- 描述生成图上的「按账号已配置顺序」指总管理在「LLM 渠道配置」里排好的全站顺序。员工端不填写 key。
- 邀请码展示图上的码形是示例，不规定码的格式。编辑菜图上的示例菜名和所选类别只是样子。
- 侧栏里出现的邀请码、导出、审计、邮箱、阈值、找回、LLM 按权限显隐，不是每个账号都看得到。各张总管图的侧栏高亮不完全一样，以当前页为准。登录页没有左侧栏。
- 数据导出图勾选的是食堂、菜谱、报餐、评分、评语、审计。不含密码哈希、TOTP、会话令牌、LLM key。图上的导出时间和「处理中」只是样子。
- 审计日志图上的姓名、动作和对象只是样子。不把「删除用户」写成一期动作。本页不能删除未满 6 个月的记录。
- 邮箱设置图上的开关，以及关闭后无 magic link、不强制验证邮箱、可不配 SMTP，与现有规则一致。服务器地址、端口、账号、密码仍在配置文件里，不因这张图改成在页面上写入。
- 强提醒阈值是独立页，不放在食堂的强提醒列表上，也不做成邀请码列表上的弹层。图上的空值不代表仓库写死数字。保存后只影响之后的提醒，不自动下架。
- 账号找回按账号重置密码或清除失效登录材料。图上的「申请时间」不增加员工自助申请。不展示密码哈希和密钥。
- LLM 渠道配置图上的星号只表示 key 已配置，不回显原文。渠道至少仍包括 OpenAI 旧接口、OpenAI 新接口、Anthropic，以及带 base URL 的兼容接口。图上只画了两条，不缩小这个范围。拖动顺序、添加渠道，以及员工只用于描述生成模板，与现有规则一致。

## 画面是否都有图

第 11 节列出的画面都已有参考图。评语、截止后改单虽有旧稿，导航仍以本节主参考的左侧栏为准。

班制参考图里的「长白班五休二」一期禁止实现。
