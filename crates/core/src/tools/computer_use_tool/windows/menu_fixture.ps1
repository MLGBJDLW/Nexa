if ($env:NEXA_FIXTURE_WPF_MENU -eq '1') {
    Add-Type -AssemblyName PresentationFramework
    $other = [Windows.Window]::new()
    $other.Title = 'Nexa unrelated menu fixture'
    $other.Width = 250
    $other.Height = 100
    $other.Show()
    $form = [Windows.Window]::new()
    $form.Title = 'Nexa owned menu fixture'
    $form.WindowStartupLocation = 'CenterScreen'
    $form.Width = 420
    $form.Height = 230
    $panel = [Windows.Controls.StackPanel]::new()
    $tree = [Windows.Controls.TreeView]::new()
    $folder = [Windows.Controls.TreeViewItem]::new()
    $folder.Header = 'Folder'
    [void]$folder.Items.Add('Child')
    [void]$tree.Items.Add($folder)
    [void]$panel.Children.Add($tree)
    $button = [Windows.Controls.Button]::new()
    $button.Content = 'Open commands'
    [void]$panel.Children.Add($button)
    $form.Content = $panel
    $menu = [Windows.Controls.ContextMenu]::new()
    $apply = [Windows.Controls.MenuItem]::new()
    $apply.Header = 'Apply choice'
    $apply.Add_Click({ $form.Title = 'Nexa menu choice applied' })
    [void]$menu.Items.Add($apply)
    $menu.PlacementTarget = $button
    $menu.Placement = 'Right'
    $button.Add_Click({ $menu.IsOpen = $true })
    [void]$form.ShowDialog()
    $other.Close()
    return
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class FixtureMenu {
    [DllImport("user32")] public static extern bool SetProcessDPIAware();
    [DllImport("user32")] public static extern IntPtr CreatePopupMenu();
    [DllImport("user32", CharSet=CharSet.Unicode)] public static extern bool AppendMenu(IntPtr menu, uint flags, UIntPtr id, string label);
    [DllImport("user32")] public static extern uint TrackPopupMenu(IntPtr menu, uint flags, int x, int y, int reserved, IntPtr window, IntPtr rect);
    [DllImport("user32")] public static extern bool DestroyMenu(IntPtr menu);
}
"@
[void][FixtureMenu]::SetProcessDPIAware()
$other = [Windows.Forms.Form]::new()
$other.Text = 'Nexa unrelated menu fixture'
$other.Show()
$form = [Windows.Forms.Form]::new()
$form.Text = 'Nexa owned menu fixture'
$form.StartPosition = 'CenterScreen'
$form.ClientSize = [Drawing.Size]::new(420,230)
$tree = [Windows.Forms.TreeView]::new()
$tree.SetBounds(15,15,190,190)
$node = $tree.Nodes.Add('Folder')
[void]$node.Nodes.Add('Child')
$button = [Windows.Forms.Button]::new()
$button.Text = 'Open commands'
$button.SetBounds(230,50,150,48)
$form.Controls.AddRange(@($tree,$button))
$timer = [Windows.Forms.Timer]::new()
$timer.Interval = 50
$button.Add_Click({ $timer.Start() })
$timer.Add_Tick({
    $timer.Stop()
    $menu = [FixtureMenu]::CreatePopupMenu()
    try {
        [void][FixtureMenu]::AppendMenu($menu,0,[UIntPtr]::new([uint32]101),'Keep editing')
        [void][FixtureMenu]::AppendMenu($menu,0,[UIntPtr]::new([uint32]102),'Apply choice')
        $point = $form.PointToScreen([Drawing.Point]::new($form.ClientSize.Width-3,90))
        $choice = [FixtureMenu]::TrackPopupMenu($menu,256,$point.X,$point.Y,0,$form.Handle,[IntPtr]::Zero)
        if ($choice -eq 102) { $form.Text = 'Nexa menu choice applied' }
    } finally { [void][FixtureMenu]::DestroyMenu($menu) }
})
$form.Add_Shown({ $form.Activate() })
[Windows.Forms.Application]::Run($form)
$other.Dispose()
